use std::collections::BTreeMap;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Utc};
use futures_core::Stream;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::error::AppError;
use crate::model::usage::{
    DailyUsageRow, UsageBreakdownRow, UsageBucket, UsageDateRange, UsageEvent, UsageFilters,
    UsageGranularity, UsageGroupBy, UsageMetrics, UsagePricingUpdate, UsageReport,
    UsageReportQuery, UsageTokens,
};
use crate::service::usage_pricing::{
    PricingEngine, PricingSnapshot, fetch_remote_pricing_snapshot,
};
use crate::store::usage_store::UsageStore;

const QUEUE_CAPACITY: usize = 4096;
const WRITE_BATCH_SIZE: usize = 128;
const WRITE_BATCH_DELAY: Duration = Duration::from_millis(200);
const JSON_LIMIT: usize = 16 * 1024 * 1024;
const SSE_LINE_LIMIT: usize = 256 * 1024;
const RETENTION_BATCH_SIZE: i64 = 5_000;
const PRICING_BACKFILL_BATCH_SIZE: i64 = 250;
const PRICING_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const DECODE_BUFFER_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageContentEncoding {
    Identity,
    Gzip,
    Deflate,
    Brotli,
    Zstd,
    Unsupported,
}

impl UsageContentEncoding {
    pub fn parse(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Identity;
        };
        let mut encodings = value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(encoding) = encodings.next() else {
            return Self::Identity;
        };
        if encodings.next().is_some() {
            return Self::Unsupported;
        }
        match encoding.to_ascii_lowercase().as_str() {
            "identity" => Self::Identity,
            "gzip" | "x-gzip" => Self::Gzip,
            "deflate" => Self::Deflate,
            "br" => Self::Brotli,
            "zstd" => Self::Zstd,
            _ => Self::Unsupported,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsageAttempt {
    pub attempt_id: String,
    pub occurred_at_utc: DateTime<Utc>,
    pub account_id: i64,
    pub api_token_id: i64,
    pub request_model: String,
    pub upstream_request_id: Option<String>,
    pub http_status: u16,
    pub is_stream: bool,
    pub content_encoding: UsageContentEncoding,
}

impl UsageAttempt {
    pub fn begin(
        account_id: i64,
        api_token_id: i64,
        request_model: String,
        is_stream: bool,
    ) -> Self {
        Self {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            occurred_at_utc: Utc::now(),
            account_id,
            api_token_id,
            request_model,
            upstream_request_id: None,
            http_status: 0,
            is_stream,
            content_encoding: UsageContentEncoding::Identity,
        }
    }

    pub fn complete_response(
        &mut self,
        http_status: u16,
        upstream_request_id: Option<String>,
        content_encoding: UsageContentEncoding,
    ) {
        self.http_status = http_status;
        self.upstream_request_id = upstream_request_id;
        self.content_encoding = content_encoding;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageIngestionHealth {
    pub since_utc: DateTime<Utc>,
    pub queue_depth: usize,
    pub observed_total: u64,
    pub persisted_total: u64,
    pub duplicate_total: u64,
    pub queue_dropped_total: u64,
    pub write_failed_total: u64,
    pub parse_failed_total: u64,
    pub parse_oversize_total: u64,
}

#[derive(Default)]
struct UsageHealthCounters {
    observed: AtomicU64,
    persisted: AtomicU64,
    duplicate: AtomicU64,
    queue_dropped: AtomicU64,
    write_failed: AtomicU64,
    parse_failed: AtomicU64,
    parse_oversize: AtomicU64,
}

pub struct UsageService {
    store: Arc<UsageStore>,
    pricing: Arc<RwLock<PricingEngine>>,
    sender: mpsc::Sender<UsageEvent>,
    health: Arc<UsageHealthCounters>,
    since_utc: DateTime<Utc>,
}

impl UsageService {
    pub async fn start(store: Arc<UsageStore>, pricing: PricingEngine) -> Arc<Self> {
        Self::start_inner(store, pricing, true).await
    }

    #[cfg(test)]
    async fn start_without_pricing_refresh(
        store: Arc<UsageStore>,
        pricing: PricingEngine,
    ) -> Arc<Self> {
        Self::start_inner(store, pricing, false).await
    }

    async fn start_inner(
        store: Arc<UsageStore>,
        mut pricing: PricingEngine,
        refresh_pricing: bool,
    ) -> Arc<Self> {
        match store.load_pricing_snapshot().await {
            Ok(Some(json)) => match PricingSnapshot::from_json(&json)
                .and_then(|snapshot| pricing.with_snapshot(&snapshot))
            {
                Ok(cached) => {
                    info!("loaded cached usage pricing version {}", cached.version());
                    pricing = cached;
                }
                Err(error) => warn!("cached usage pricing ignored: {}", error),
            },
            Ok(None) => {}
            Err(error) => warn!("load cached usage pricing failed: {}", error),
        }
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let health = Arc::new(UsageHealthCounters::default());
        let pricing = Arc::new(RwLock::new(pricing));
        let service = Arc::new(Self {
            store: store.clone(),
            pricing: pricing.clone(),
            sender,
            health: health.clone(),
            since_utc: Utc::now(),
        });
        tokio::spawn(run_writer(store.clone(), pricing.clone(), receiver, health));
        tokio::spawn(run_retention(store.clone()));
        if refresh_pricing {
            tokio::spawn(run_pricing_refresh(store, pricing));
        }
        service
    }

    pub fn observe_stream<S, E>(
        self: &Arc<Self>,
        stream: S,
        attempt: UsageAttempt,
    ) -> UsageObservedStream<S>
    where
        S: Stream<Item = Result<Bytes, E>>,
    {
        UsageObservedStream {
            inner: Box::pin(stream),
            observer: Some(ResponseObserver::new(self.clone(), attempt)),
        }
    }

    pub fn health(&self) -> UsageIngestionHealth {
        UsageIngestionHealth {
            since_utc: self.since_utc,
            queue_depth: QUEUE_CAPACITY.saturating_sub(self.sender.capacity()),
            observed_total: self.health.observed.load(Ordering::Relaxed),
            persisted_total: self.health.persisted.load(Ordering::Relaxed),
            duplicate_total: self.health.duplicate.load(Ordering::Relaxed),
            queue_dropped_total: self.health.queue_dropped.load(Ordering::Relaxed),
            write_failed_total: self.health.write_failed.load(Ordering::Relaxed),
            parse_failed_total: self.health.parse_failed.load(Ordering::Relaxed),
            parse_oversize_total: self.health.parse_oversize.load(Ordering::Relaxed),
        }
    }

    pub async fn dimensions(
        &self,
        min_sg_day: i32,
    ) -> Result<crate::store::usage_store::UsageDimensions, crate::error::AppError> {
        self.store.dimensions(min_sg_day).await
    }

    pub async fn report(&self, query: UsageReportQuery) -> Result<UsageReport, AppError> {
        let filters = UsageFilters {
            start_sg_day: sg_day_from_date(query.start_date),
            end_sg_day: sg_day_from_date(query.end_date),
            account_id: query.account_id,
            api_token_id: query.api_token_id,
            model: query.model.clone(),
            group_by: query.group_by,
        };
        let daily_rows = self.store.aggregate_daily(&filters).await?;
        let labels = self
            .group_labels(query.group_by, filters.start_sg_day)
            .await?;

        let mut summary = UsageMetrics::default();
        let mut metrics_by_day: BTreeMap<i32, UsageMetrics> = BTreeMap::new();
        let mut breakdown: BTreeMap<String, UsageMetrics> = BTreeMap::new();
        for row in &daily_rows {
            summary.add_assign(&row.metrics);
            metrics_by_day
                .entry(row.sg_day)
                .or_default()
                .add_assign(&row.metrics);
            breakdown
                .entry(group_key(row, query.group_by))
                .or_default()
                .add_assign(&row.metrics);
        }

        let buckets = build_buckets(
            query.start_date,
            query.end_date,
            query.granularity,
            &metrics_by_day,
        );
        let mut breakdown: Vec<UsageBreakdownRow> = breakdown
            .into_iter()
            .map(|(key, metrics)| UsageBreakdownRow {
                label: labels.get(&key).cloned().unwrap_or_else(|| key.clone()),
                key,
                metrics,
            })
            .collect();
        breakdown.sort_by(|left, right| {
            right
                .metrics
                .known_cost_nano_usd
                .cmp(&left.metrics.known_cost_nano_usd)
                .then_with(|| {
                    right
                        .metrics
                        .tokens
                        .total()
                        .cmp(&left.metrics.tokens.total())
                })
                .then_with(|| left.label.cmp(&right.label))
        });

        Ok(UsageReport {
            timezone: "Asia/Singapore",
            granularity: query.granularity,
            range: UsageDateRange {
                start_date: query.start_date,
                end_date: query.end_date,
            },
            summary,
            buckets,
            breakdown,
        })
    }

    async fn group_labels(
        &self,
        group_by: UsageGroupBy,
        min_sg_day: i32,
    ) -> Result<BTreeMap<String, String>, AppError> {
        if group_by == UsageGroupBy::Model {
            return Ok(BTreeMap::new());
        }
        let dimensions = self.store.dimensions(min_sg_day).await?;
        let options = match group_by {
            UsageGroupBy::Account => dimensions.accounts,
            UsageGroupBy::ApiToken => dimensions.api_tokens,
            UsageGroupBy::Model => unreachable!("handled above"),
        };
        Ok(options
            .into_iter()
            .map(|option| (option.id.to_string(), option.label))
            .collect())
    }

    fn submit(&self, attempt: UsageAttempt, parsed: ParsedUsage) {
        if !parsed.has_usage {
            return;
        }
        let model = parsed
            .model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(attempt.request_model);
        let priced = self
            .pricing
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .price(&model, &parsed.tokens);
        let dedup_key = if let Some(message_id) = parsed.message_id.as_deref() {
            format!("anthropic:message:{message_id}")
        } else if let Some(request_id) = attempt.upstream_request_id.as_deref() {
            format!("anthropic:request:{request_id}")
        } else {
            format!("attempt:{}", attempt.attempt_id)
        };
        let event = UsageEvent {
            dedup_key,
            upstream_message_id: parsed.message_id,
            upstream_request_id: attempt.upstream_request_id,
            occurred_at_utc: attempt.occurred_at_utc,
            sg_day: sg_day_from_utc(attempt.occurred_at_utc),
            account_id: attempt.account_id,
            api_token_id: attempt.api_token_id,
            model,
            tokens: parsed.tokens,
            costs: priced.costs,
            pricing_version: priced.pricing_version,
            pricing_model_key: priced.pricing_model_key,
            http_status: attempt.http_status,
            is_stream: attempt.is_stream,
        };
        self.health.observed.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = self.sender.try_send(event) {
            let kind = match error {
                mpsc::error::TrySendError::Full(_) => "full",
                mpsc::error::TrySendError::Closed(_) => "closed",
            };
            let count = self.health.queue_dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if count == 1 || count % 100 == 0 {
                warn!(
                    "usage event dropped because writer queue is unavailable (count={}, kind={})",
                    count, kind
                );
            }
        }
    }

    fn record_parse_failure(&self, oversize: bool) {
        let counter = if oversize {
            &self.health.parse_oversize
        } else {
            &self.health.parse_failed
        };
        let count = counter.fetch_add(1, Ordering::Relaxed) + 1;
        if count == 1 || count % 100 == 0 {
            warn!(
                "usage response could not be parsed (count={}, kind={})",
                count,
                if oversize { "oversize" } else { "invalid" }
            );
        }
    }
}

pub struct UsageObservedStream<S> {
    inner: Pin<Box<S>>,
    observer: Option<ResponseObserver>,
}

impl<S, E> Stream for UsageObservedStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if let Some(observer) = this.observer.as_mut() {
                    observer.feed(&bytes);
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                if let Some(mut observer) = this.observer.take() {
                    observer.finish(false);
                }
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                if let Some(mut observer) = this.observer.take() {
                    observer.finish(false);
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for UsageObservedStream<S> {
    fn drop(&mut self) {
        if let Some(mut observer) = self.observer.take() {
            observer.finish(true);
        }
    }
}

struct ResponseObserver {
    service: Arc<UsageService>,
    attempt: Option<UsageAttempt>,
    parser: EncodedResponseParser,
}

impl ResponseObserver {
    fn new(service: Arc<UsageService>, attempt: UsageAttempt) -> Self {
        let response_parser = if attempt.is_stream {
            ResponseParser::Sse(SseParser::default())
        } else {
            ResponseParser::Json(JsonParser::default())
        };
        let parser = EncodedResponseParser::new(response_parser, attempt.content_encoding);
        Self {
            service,
            attempt: Some(attempt),
            parser,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        if self.attempt.is_none() {
            return;
        }
        self.parser.feed(bytes);
        if self.parser.is_complete() && self.parser.can_finalize_early() {
            self.finalize(false);
        }
    }

    fn finish(&mut self, client_dropped: bool) {
        if self.attempt.is_none() {
            return;
        }
        self.parser.finish(client_dropped);
        self.finalize(true);
    }

    fn finalize(&mut self, terminal: bool) {
        if self.parser.is_invalid() {
            let attempt = self.attempt.take();
            if attempt.is_some() {
                self.service.record_parse_failure(self.parser.is_oversize());
            }
            return;
        }
        if let Some(parsed) = self.parser.parsed() {
            if parsed.has_usage && (terminal || self.parser.is_complete()) {
                if let Some(attempt) = self.attempt.take() {
                    self.service.submit(attempt, parsed);
                }
            } else if terminal {
                self.attempt.take();
            }
        } else if terminal {
            self.attempt.take();
        }
    }
}

struct ParserSink {
    parser: ResponseParser,
}

impl Write for ParserSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.parser.feed(bytes);
        if self.parser.is_invalid() {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "usage parser rejected decoded response",
            ))
        } else {
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ZstdParserDecoder {
    decoder: zstd::stream::raw::Decoder<'static>,
    sink: ParserSink,
    frame_complete: bool,
}

impl ZstdParserDecoder {
    fn new(sink: ParserSink) -> io::Result<Self> {
        Ok(Self {
            decoder: zstd::stream::raw::Decoder::new()?,
            sink,
            frame_complete: false,
        })
    }

    fn feed(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        use zstd::stream::raw::Operation;

        if self.frame_complete && !bytes.is_empty() {
            self.decoder.reinit()?;
            self.frame_complete = false;
        }
        loop {
            let mut decoded = [0_u8; DECODE_BUFFER_SIZE];
            let status = self.decoder.run_on_buffers(bytes, &mut decoded)?;
            if status.bytes_written > 0 {
                self.sink.write_all(&decoded[..status.bytes_written])?;
            }
            bytes = &bytes[status.bytes_read..];
            self.frame_complete = status.remaining == 0;
            if self.frame_complete && !bytes.is_empty() {
                self.decoder.reinit()?;
                self.frame_complete = false;
            } else if bytes.is_empty() {
                if status.bytes_written == decoded.len() {
                    continue;
                }
                break;
            } else if status.bytes_read == 0 && status.bytes_written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "zstd decoder made no progress",
                ));
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.frame_complete {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete zstd response",
            ))
        }
    }
}

struct DeflateParserDecoder {
    state: Option<DeflateDecoderState>,
}

enum DeflateDecoderState {
    Pending { prefix: Vec<u8>, sink: ParserSink },
    Zlib(flate2::write::ZlibDecoder<ParserSink>),
    Raw(flate2::write::DeflateDecoder<ParserSink>),
}

impl DeflateParserDecoder {
    fn new(sink: ParserSink) -> Self {
        Self {
            state: Some(DeflateDecoderState::Pending {
                prefix: Vec::with_capacity(2),
                sink,
            }),
        }
    }

    fn parser(&self) -> &ResponseParser {
        match self
            .state
            .as_ref()
            .expect("deflate state is always present")
        {
            DeflateDecoderState::Pending { sink, .. } => &sink.parser,
            DeflateDecoderState::Zlib(decoder) => &decoder.get_ref().parser,
            DeflateDecoderState::Raw(decoder) => &decoder.get_ref().parser,
        }
    }

    fn parser_mut(&mut self) -> &mut ResponseParser {
        match self
            .state
            .as_mut()
            .expect("deflate state is always present")
        {
            DeflateDecoderState::Pending { sink, .. } => &mut sink.parser,
            DeflateDecoderState::Zlib(decoder) => &mut decoder.get_mut().parser,
            DeflateDecoderState::Raw(decoder) => &mut decoder.get_mut().parser,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        let is_pending = matches!(self.state, Some(DeflateDecoderState::Pending { .. }));
        if is_pending {
            let state = self.state.take().expect("deflate state is always present");
            let DeflateDecoderState::Pending { mut prefix, sink } = state else {
                unreachable!("checked above")
            };
            let needed = 2_usize.saturating_sub(prefix.len());
            let consumed = needed.min(bytes.len());
            prefix.extend_from_slice(&bytes[..consumed]);
            if prefix.len() < 2 {
                self.state = Some(DeflateDecoderState::Pending { prefix, sink });
                return Ok(());
            }
            let is_zlib = prefix[0] & 0x0f == 8
                && prefix[0] >> 4 <= 7
                && u16::from_be_bytes([prefix[0], prefix[1]]) % 31 == 0;
            self.state = Some(if is_zlib {
                DeflateDecoderState::Zlib(flate2::write::ZlibDecoder::new(sink))
            } else {
                DeflateDecoderState::Raw(flate2::write::DeflateDecoder::new(sink))
            });
            self.feed(&prefix)?;
            return self.feed(&bytes[consumed..]);
        }

        match self
            .state
            .as_mut()
            .expect("deflate state is always present")
        {
            DeflateDecoderState::Zlib(decoder) => decoder.write_all(bytes),
            DeflateDecoderState::Raw(decoder) => decoder.write_all(bytes),
            DeflateDecoderState::Pending { .. } => unreachable!("handled above"),
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        match self
            .state
            .as_mut()
            .expect("deflate state is always present")
        {
            DeflateDecoderState::Pending { .. } => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete deflate response",
            )),
            DeflateDecoderState::Zlib(decoder) => decoder.try_finish(),
            DeflateDecoderState::Raw(decoder) => decoder.try_finish(),
        }
    }
}

enum Decoder {
    Identity(ParserSink),
    Gzip(flate2::write::GzDecoder<ParserSink>),
    Deflate(DeflateParserDecoder),
    Brotli(brotli::DecompressorWriter<ParserSink>),
    Zstd(ZstdParserDecoder),
}

impl Decoder {
    fn parser(&self) -> &ResponseParser {
        match self {
            Self::Identity(sink) => &sink.parser,
            Self::Gzip(decoder) => &decoder.get_ref().parser,
            Self::Deflate(decoder) => decoder.parser(),
            Self::Brotli(decoder) => &decoder.get_ref().parser,
            Self::Zstd(decoder) => &decoder.sink.parser,
        }
    }

    fn parser_mut(&mut self) -> &mut ResponseParser {
        match self {
            Self::Identity(sink) => &mut sink.parser,
            Self::Gzip(decoder) => &mut decoder.get_mut().parser,
            Self::Deflate(decoder) => decoder.parser_mut(),
            Self::Brotli(decoder) => &mut decoder.get_mut().parser,
            Self::Zstd(decoder) => &mut decoder.sink.parser,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Identity(sink) => sink.write_all(bytes),
            Self::Gzip(decoder) => decoder.write_all(bytes),
            Self::Deflate(decoder) => decoder.feed(bytes),
            Self::Brotli(decoder) => decoder.write_all(bytes),
            Self::Zstd(decoder) => decoder.feed(bytes),
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Identity(_) => Ok(()),
            Self::Gzip(decoder) => decoder.try_finish(),
            Self::Deflate(decoder) => decoder.finish(),
            Self::Brotli(decoder) => decoder.close(),
            Self::Zstd(decoder) => decoder.finish(),
        }
    }
}

struct EncodedResponseParser {
    decoder: Decoder,
    decode_failed: bool,
    requires_terminal_validation: bool,
}

impl EncodedResponseParser {
    fn new(parser: ResponseParser, encoding: UsageContentEncoding) -> Self {
        let sink = ParserSink { parser };
        let requires_terminal_validation = encoding != UsageContentEncoding::Identity;
        let (decoder, decode_failed) = match encoding {
            UsageContentEncoding::Identity => (Decoder::Identity(sink), false),
            UsageContentEncoding::Gzip => {
                (Decoder::Gzip(flate2::write::GzDecoder::new(sink)), false)
            }
            UsageContentEncoding::Deflate => {
                (Decoder::Deflate(DeflateParserDecoder::new(sink)), false)
            }
            UsageContentEncoding::Brotli => (
                Decoder::Brotli(brotli::DecompressorWriter::new(sink, DECODE_BUFFER_SIZE)),
                false,
            ),
            UsageContentEncoding::Zstd => match ZstdParserDecoder::new(sink) {
                Ok(decoder) => (Decoder::Zstd(decoder), false),
                Err(_) => (
                    Decoder::Identity(ParserSink {
                        parser: ResponseParser::Json(JsonParser::default()),
                    }),
                    true,
                ),
            },
            UsageContentEncoding::Unsupported => (Decoder::Identity(sink), true),
        };
        Self {
            decoder,
            decode_failed,
            requires_terminal_validation,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        if !self.decode_failed && self.decoder.feed(bytes).is_err() {
            self.decode_failed = true;
        }
    }

    fn finish(&mut self, allow_incomplete_transport: bool) {
        let usage_observed = self.decoder.parser().parsed().is_some();
        if !self.decode_failed
            && self.decoder.finish().is_err()
            && !(allow_incomplete_transport && usage_observed)
        {
            self.decode_failed = true;
        }
        if !self.decode_failed {
            self.decoder.parser_mut().finish();
        }
    }

    fn parsed(&self) -> Option<ParsedUsage> {
        self.decoder.parser().parsed()
    }

    fn is_complete(&self) -> bool {
        !self.decode_failed && self.decoder.parser().is_complete()
    }

    fn can_finalize_early(&self) -> bool {
        !self.requires_terminal_validation
    }

    fn is_invalid(&self) -> bool {
        self.decode_failed || self.decoder.parser().is_invalid()
    }

    fn is_oversize(&self) -> bool {
        self.decoder.parser().is_oversize()
    }
}

enum ResponseParser {
    Json(JsonParser),
    Sse(SseParser),
}

impl ResponseParser {
    fn feed(&mut self, bytes: &[u8]) {
        match self {
            Self::Json(parser) => parser.feed(bytes),
            Self::Sse(parser) => parser.feed(bytes),
        }
    }

    fn finish(&mut self) {
        match self {
            Self::Json(parser) => parser.finish(),
            Self::Sse(parser) => parser.finish(),
        }
    }

    fn parsed(&self) -> Option<ParsedUsage> {
        match self {
            Self::Json(parser) => parser.parsed.clone(),
            Self::Sse(parser) => parser.parsed(),
        }
    }

    fn is_complete(&self) -> bool {
        matches!(self, Self::Sse(parser) if parser.complete)
    }

    fn is_invalid(&self) -> bool {
        match self {
            Self::Json(parser) => parser.invalid,
            Self::Sse(parser) => parser.invalid,
        }
    }

    fn is_oversize(&self) -> bool {
        match self {
            Self::Json(parser) => parser.oversize,
            Self::Sse(parser) => parser.oversize,
        }
    }
}

#[derive(Default)]
struct JsonParser {
    buffer: Vec<u8>,
    parsed: Option<ParsedUsage>,
    invalid: bool,
    oversize: bool,
}

impl JsonParser {
    fn feed(&mut self, bytes: &[u8]) {
        if self.invalid {
            return;
        }
        if self.buffer.len().saturating_add(bytes.len()) > JSON_LIMIT {
            self.buffer.clear();
            self.invalid = true;
            self.oversize = true;
            return;
        }
        self.buffer.extend_from_slice(bytes);
    }

    fn finish(&mut self) {
        if self.invalid || self.parsed.is_some() || self.buffer.is_empty() {
            return;
        }
        match serde_json::from_slice::<Value>(&self.buffer) {
            Ok(value) => match parse_json_response(&value) {
                Ok(parsed) => self.parsed = Some(parsed),
                Err(()) => self.invalid = true,
            },
            Err(_) => self.invalid = true,
        }
        self.buffer.clear();
    }
}

#[derive(Default)]
struct SseParser {
    line: Vec<u8>,
    message_id: Option<String>,
    model: Option<String>,
    tokens: UsageTokens,
    has_usage: bool,
    complete: bool,
    invalid: bool,
    oversize: bool,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) {
        if self.invalid || self.complete {
            return;
        }
        for &byte in bytes {
            if byte == b'\n' {
                self.process_line();
                self.line.clear();
                if self.invalid || self.complete {
                    return;
                }
            } else {
                if self.line.len() >= SSE_LINE_LIMIT {
                    self.line.clear();
                    self.invalid = true;
                    self.oversize = true;
                    return;
                }
                self.line.push(byte);
            }
        }
    }

    fn finish(&mut self) {
        if !self.invalid && !self.complete && !self.line.is_empty() {
            self.process_line();
            self.line.clear();
        }
    }

    fn process_line(&mut self) {
        let line = self.line.strip_suffix(b"\r").unwrap_or(&self.line);
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        if data.is_empty() || data == b"[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(data) else {
            self.invalid = true;
            return;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let Some(message) = value.get("message") else {
                    self.invalid = true;
                    return;
                };
                self.message_id = string_field(message, "id").or(self.message_id.take());
                self.model = string_field(message, "model").or(self.model.take());
                if let Some(usage) = message.get("usage") {
                    match parse_usage_patch(usage) {
                        Ok(patch) => {
                            self.has_usage |= patch.has_any();
                            patch.apply(&mut self.tokens);
                        }
                        Err(()) => self.invalid = true,
                    }
                }
            }
            Some("message_delta") => {
                if let Some(usage) = value.get("usage") {
                    match parse_usage_patch(usage) {
                        Ok(patch) => {
                            self.has_usage |= patch.has_any();
                            patch.apply(&mut self.tokens);
                        }
                        Err(()) => self.invalid = true,
                    }
                }
            }
            Some("message_stop") => self.complete = true,
            _ => {}
        }
    }

    fn parsed(&self) -> Option<ParsedUsage> {
        self.has_usage.then(|| ParsedUsage {
            message_id: self.message_id.clone(),
            model: self.model.clone(),
            tokens: self.tokens.clone(),
            has_usage: true,
        })
    }
}

#[derive(Debug, Clone)]
struct ParsedUsage {
    message_id: Option<String>,
    model: Option<String>,
    tokens: UsageTokens,
    has_usage: bool,
}

#[derive(Default)]
struct UsagePatch {
    input: Option<i64>,
    output: Option<i64>,
    cache_creation_5m: Option<i64>,
    cache_creation_1h: Option<i64>,
    cache_read: Option<i64>,
}

impl UsagePatch {
    fn has_any(&self) -> bool {
        self.input.is_some()
            || self.output.is_some()
            || self.cache_creation_5m.is_some()
            || self.cache_creation_1h.is_some()
            || self.cache_read.is_some()
    }

    fn apply(self, tokens: &mut UsageTokens) {
        if let Some(value) = self.input {
            tokens.input = value;
        }
        if let Some(value) = self.output {
            tokens.output = value;
        }
        if let Some(value) = self.cache_creation_5m {
            tokens.cache_creation_5m = value;
        }
        if let Some(value) = self.cache_creation_1h {
            tokens.cache_creation_1h = value;
        }
        if let Some(value) = self.cache_read {
            tokens.cache_read = value;
        }
    }
}

fn parse_json_response(value: &Value) -> Result<ParsedUsage, ()> {
    let patch = value
        .get("usage")
        .map(parse_usage_patch)
        .transpose()?
        .unwrap_or_default();
    let has_usage = value.get("usage").is_some() && patch.has_any();
    let mut tokens = UsageTokens::default();
    patch.apply(&mut tokens);
    Ok(ParsedUsage {
        message_id: string_field(value, "id"),
        model: string_field(value, "model"),
        tokens,
        has_usage,
    })
}

fn parse_usage_patch(usage: &Value) -> Result<UsagePatch, ()> {
    let Some(object) = usage.as_object() else {
        return Err(());
    };
    let mut patch = UsagePatch {
        input: optional_token(object.get("input_tokens"))?,
        output: optional_token(object.get("output_tokens"))?,
        cache_read: optional_token(object.get("cache_read_input_tokens"))?,
        ..UsagePatch::default()
    };
    let breakdown = object.get("cache_creation").and_then(Value::as_object);
    if let Some(breakdown) = breakdown {
        patch.cache_creation_5m = optional_token(breakdown.get("ephemeral_5m_input_tokens"))?;
        patch.cache_creation_1h = optional_token(breakdown.get("ephemeral_1h_input_tokens"))?;
    } else {
        patch.cache_creation_5m = optional_token(object.get("cache_creation_input_tokens"))?;
    }
    Ok(patch)
}

fn optional_token(value: Option<&Value>) -> Result<Option<i64>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|value| *value >= 0)
            .map(Some)
            .ok_or(()),
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn sg_day_from_utc(timestamp: DateTime<Utc>) -> i32 {
    let local_date = (timestamp + ChronoDuration::hours(8)).date_naive();
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch date");
    i32::try_from(local_date.signed_duration_since(epoch).num_days()).unwrap_or(i32::MAX)
}

pub fn sg_day_from_date(date: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch date");
    i32::try_from(date.signed_duration_since(epoch).num_days()).unwrap_or(i32::MAX)
}

pub fn date_from_sg_day(day: i32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .expect("valid epoch date")
        .checked_add_signed(ChronoDuration::days(i64::from(day)))
        .expect("usage day is in supported date range")
}

fn group_key(row: &DailyUsageRow, group_by: UsageGroupBy) -> String {
    match group_by {
        UsageGroupBy::Account | UsageGroupBy::ApiToken => row.group_id.to_string(),
        UsageGroupBy::Model => row.group_text.clone(),
    }
}

fn build_buckets(
    start_date: NaiveDate,
    end_date: NaiveDate,
    granularity: UsageGranularity,
    metrics_by_day: &BTreeMap<i32, UsageMetrics>,
) -> Vec<UsageBucket> {
    let mut buckets: BTreeMap<NaiveDate, (NaiveDate, NaiveDate, UsageMetrics)> = BTreeMap::new();
    let mut date = start_date;
    while date <= end_date {
        let natural_start = bucket_start(date, granularity);
        let entry = buckets
            .entry(natural_start)
            .or_insert_with(|| (date, date, UsageMetrics::default()));
        entry.1 = date;
        if let Some(metrics) = metrics_by_day.get(&sg_day_from_date(date)) {
            entry.2.add_assign(metrics);
        }
        date = date
            .checked_add_signed(ChronoDuration::days(1))
            .expect("validated usage range can advance by one day");
    }

    buckets
        .into_iter()
        .map(
            |(natural_start, (covered_start, covered_end, metrics))| UsageBucket {
                key: natural_start.format("%Y-%m-%d").to_string(),
                start_date: covered_start,
                end_date: covered_end,
                start_at_utc: singapore_midnight_utc(covered_start),
                end_at_utc_exclusive: singapore_midnight_utc(
                    covered_end
                        .checked_add_signed(ChronoDuration::days(1))
                        .expect("validated usage range has an exclusive end"),
                ),
                metrics,
            },
        )
        .collect()
}

fn bucket_start(date: NaiveDate, granularity: UsageGranularity) -> NaiveDate {
    match granularity {
        UsageGranularity::Day => date,
        UsageGranularity::Week => date
            .checked_sub_signed(ChronoDuration::days(i64::from(
                date.weekday().num_days_from_monday(),
            )))
            .expect("week start is representable"),
        UsageGranularity::Month => {
            NaiveDate::from_ymd_opt(date.year(), date.month(), 1).expect("valid month start")
        }
    }
}

fn singapore_midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    date.and_hms_opt(0, 0, 0).expect("valid midnight").and_utc() - ChronoDuration::hours(8)
}

async fn run_writer(
    store: Arc<UsageStore>,
    pricing: Arc<RwLock<PricingEngine>>,
    mut receiver: mpsc::Receiver<UsageEvent>,
    health: Arc<UsageHealthCounters>,
) {
    while let Some(first) = receiver.recv().await {
        let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);
        batch.push(first);
        let deadline = tokio::time::Instant::now() + WRITE_BATCH_DELAY;
        while batch.len() < WRITE_BATCH_SIZE {
            match tokio::time::timeout_at(deadline, receiver.recv()).await {
                Ok(Some(event)) => batch.push(event),
                Ok(None) | Err(_) => break,
            }
        }

        let current_pricing = pricing
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for event in &mut batch {
            if !event.costs.complete {
                let priced = current_pricing.price(&event.model, &event.tokens);
                if priced.costs.complete {
                    event.costs = priced.costs;
                    event.pricing_version = priced.pricing_version;
                    event.pricing_model_key = priced.pricing_model_key;
                }
            }
        }

        let mut result = None;
        for retry in 0..=3_u32 {
            match store.insert_batch(&batch).await {
                Ok(counts) => {
                    result = Some(counts);
                    break;
                }
                Err(_) if retry < 3 => {
                    tokio::time::sleep(Duration::from_millis(50 * (1 << retry))).await;
                }
                Err(error) => {
                    let failed = batch.len() as u64;
                    let total = health.write_failed.fetch_add(failed, Ordering::Relaxed) + failed;
                    if total == failed || total / 100 != (total - failed) / 100 {
                        warn!(
                            "usage batch write failed permanently (events={}, total={}): {}",
                            failed, total, error
                        );
                    }
                }
            }
        }
        if let Some((inserted, duplicates)) = result {
            health.persisted.fetch_add(inserted, Ordering::Relaxed);
            health.duplicate.fetch_add(duplicates, Ordering::Relaxed);
        }
    }
}

async fn run_pricing_refresh(store: Arc<UsageStore>, pricing: Arc<RwLock<PricingEngine>>) {
    let cached = pricing
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Err(error) = backfill_unpriced_usage(&store, &cached).await {
        warn!(
            "cached/builtin pricing historical backfill failed: {}",
            error
        );
    }
    loop {
        match fetch_remote_pricing_snapshot().await {
            Ok(snapshot) => {
                let next = pricing
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .with_snapshot(&snapshot);
                match next {
                    Ok(next) => {
                        let snapshot_json = snapshot.to_json();
                        match snapshot_json {
                            Ok(json) => {
                                let version = next.version().to_string();
                                *pricing
                                    .write()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                    next.clone();
                                if let Err(error) = store
                                    .save_pricing_snapshot(&json, snapshot.version(), Utc::now())
                                    .await
                                {
                                    warn!("persist refreshed usage pricing failed: {}", error);
                                }
                                match backfill_unpriced_usage(&store, &next).await {
                                    Ok(updated) => info!(
                                        "usage pricing refreshed (version={}, backfilled={})",
                                        version, updated
                                    ),
                                    Err(error) => warn!(
                                        "usage pricing refreshed but historical backfill failed: {}",
                                        error
                                    ),
                                }
                            }
                            Err(error) => {
                                warn!("serialize refreshed usage pricing failed: {}", error)
                            }
                        }
                    }
                    Err(error) => warn!("refreshed usage pricing rejected: {}", error),
                }
            }
            Err(error) => warn!(
                "usage pricing refresh failed; keeping current cached/builtin pricing: {}",
                error
            ),
        }
        tokio::time::sleep(PRICING_REFRESH_INTERVAL).await;
    }
}

async fn backfill_unpriced_usage(
    store: &UsageStore,
    pricing: &PricingEngine,
) -> Result<u64, AppError> {
    let mut after_id = 0_i64;
    let mut updated_total = 0_u64;
    loop {
        let events = store
            .unpriced_events_after(after_id, PRICING_BACKFILL_BATCH_SIZE)
            .await?;
        if events.is_empty() {
            return Ok(updated_total);
        }
        after_id = events.last().map(|event| event.id).unwrap_or(after_id);
        let updates = events
            .into_iter()
            .filter_map(|event| {
                let priced = pricing.price(&event.model, &event.tokens);
                (priced.costs.complete && priced.pricing_model_key.is_some()).then(|| {
                    UsagePricingUpdate {
                        id: event.id,
                        costs: priced.costs,
                        pricing_version: priced.pricing_version,
                        pricing_model_key: priced
                            .pricing_model_key
                            .expect("checked pricing model key"),
                    }
                })
            })
            .collect::<Vec<_>>();
        updated_total = updated_total.saturating_add(store.apply_pricing_updates(&updates).await?);
        tokio::task::yield_now().await;
    }
}

async fn run_retention(store: Arc<UsageStore>) {
    loop {
        let min_sg_day = sg_day_from_utc(Utc::now()) - 364;
        loop {
            match store.delete_before(min_sg_day, RETENTION_BATCH_SIZE).await {
                Ok(0) => break,
                Ok(_) => tokio::task::yield_now().await,
                Err(error) => {
                    warn!("usage retention cleanup failed: {}", error);
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::usage::UsageCosts;
    use futures_util::StreamExt;
    use std::io::Write;

    const JSON_USAGE_RESPONSE: &[u8] = br#"{"id":"msg_compressed","model":"claude-sonnet-4-6","usage":{"input_tokens":11,"output_tokens":9,"cache_creation_input_tokens":7,"cache_read_input_tokens":5}}"#;
    const SSE_USAGE_RESPONSE: &[u8] = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_compressed_sse\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":11,\"cache_creation_input_tokens\":7}}}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    )
    .as_bytes();

    fn parser_for(encoding: UsageContentEncoding, is_stream: bool) -> EncodedResponseParser {
        let parser = if is_stream {
            ResponseParser::Sse(SseParser::default())
        } else {
            ResponseParser::Json(JsonParser::default())
        };
        EncodedResponseParser::new(parser, encoding)
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn incomplete_gzip_prefix(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.flush().unwrap();
        encoder.write_all(&vec![b':'; DECODE_BUFFER_SIZE]).unwrap();
        encoder.write_all(b"\n").unwrap();
        encoder.flush().unwrap();
        encoder.get_ref().clone()
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn raw_deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn brotli(bytes: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(&mut encoded, 4096, 5, 22);
            encoder.write_all(bytes).unwrap();
        }
        encoded
    }

    fn zstd(bytes: &[u8]) -> Vec<u8> {
        zstd::stream::encode_all(bytes, 3).unwrap()
    }

    #[test]
    fn json_parser_extracts_all_components_and_prefers_breakdown() {
        let value = serde_json::json!({
            "id": "msg_1",
            "model": "claude-sonnet-4-6",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_creation_input_tokens": 999,
                "cache_read_input_tokens": 30,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 40,
                    "ephemeral_1h_input_tokens": 50
                }
            }
        });
        let parsed = parse_json_response(&value).unwrap();
        assert_eq!(parsed.message_id.as_deref(), Some("msg_1"));
        assert_eq!(parsed.tokens.input, 10);
        assert_eq!(parsed.tokens.output, 20);
        assert_eq!(parsed.tokens.cache_creation_5m, 40);
        assert_eq!(parsed.tokens.cache_creation_1h, 50);
        assert_eq!(parsed.tokens.cache_read, 30);
    }

    #[test]
    fn cache_creation_total_falls_back_to_five_minutes() {
        let usage = serde_json::json!({"cache_creation_input_tokens": 17});
        let patch = parse_usage_patch(&usage).unwrap();
        assert_eq!(patch.cache_creation_5m, Some(17));
        assert_eq!(patch.cache_creation_1h, None);
    }

    #[test]
    fn sse_parser_handles_arbitrary_chunks_crlf_and_cumulative_output() {
        let payload = concat!(
            "event: message_start\r\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_s\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":11,\"cache_creation_input_tokens\":7}}}\r\n\r\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        for split in 1..payload.len() {
            let mut parser = SseParser::default();
            for chunk in payload.as_bytes().chunks(split) {
                parser.feed(chunk);
            }
            let parsed = parser.parsed().unwrap();
            assert!(parser.complete, "split={split}");
            assert_eq!(parsed.tokens.input, 11, "split={split}");
            assert_eq!(parsed.tokens.output, 9, "split={split}");
            assert_eq!(parsed.tokens.cache_creation_5m, 7, "split={split}");
        }
    }

    #[test]
    fn content_encoding_parser_accepts_supported_single_encodings() {
        assert_eq!(
            UsageContentEncoding::parse(None),
            UsageContentEncoding::Identity
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("")),
            UsageContentEncoding::Identity
        );
        assert_eq!(
            UsageContentEncoding::parse(Some(" GZip ")),
            UsageContentEncoding::Gzip
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("deflate")),
            UsageContentEncoding::Deflate
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("br")),
            UsageContentEncoding::Brotli
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("zstd")),
            UsageContentEncoding::Zstd
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("gzip, br")),
            UsageContentEncoding::Unsupported
        );
        assert_eq!(
            UsageContentEncoding::parse(Some("compress")),
            UsageContentEncoding::Unsupported
        );
    }

    #[test]
    fn compressed_json_decoders_handle_arbitrary_chunk_boundaries() {
        for (encoding, encoded) in [
            (UsageContentEncoding::Gzip, gzip(JSON_USAGE_RESPONSE)),
            (UsageContentEncoding::Deflate, deflate(JSON_USAGE_RESPONSE)),
            (
                UsageContentEncoding::Deflate,
                raw_deflate(JSON_USAGE_RESPONSE),
            ),
            (UsageContentEncoding::Brotli, brotli(JSON_USAGE_RESPONSE)),
            (UsageContentEncoding::Zstd, zstd(JSON_USAGE_RESPONSE)),
        ] {
            for chunk_size in 1..=encoded.len().min(17) {
                let mut parser = parser_for(encoding, false);
                for chunk in encoded.chunks(chunk_size) {
                    parser.feed(chunk);
                }
                parser.finish(false);
                assert!(
                    !parser.is_invalid(),
                    "encoding={encoding:?}, chunk={chunk_size}"
                );
                let parsed = parser.parsed().unwrap();
                assert_eq!(parsed.message_id.as_deref(), Some("msg_compressed"));
                assert_eq!(parsed.tokens.input, 11);
                assert_eq!(parsed.tokens.output, 9);
                assert_eq!(parsed.tokens.cache_creation_5m, 7);
                assert_eq!(parsed.tokens.cache_read, 5);
            }
        }
    }

    #[test]
    fn zstd_decoder_drains_multiple_output_buffers_from_one_input_chunk() {
        let value = serde_json::json!({
            "id": "msg_large_zstd",
            "model": "claude-sonnet-4-6",
            "padding": "x".repeat(DECODE_BUFFER_SIZE * 4),
            "usage": {"input_tokens": 11, "output_tokens": 9}
        });
        let encoded = zstd(&serde_json::to_vec(&value).unwrap());
        let mut parser = parser_for(UsageContentEncoding::Zstd, false);
        parser.feed(&encoded);
        parser.finish(false);
        assert!(!parser.is_invalid());
        let parsed = parser.parsed().unwrap();
        assert_eq!(parsed.message_id.as_deref(), Some("msg_large_zstd"));
        assert_eq!(parsed.tokens.input, 11);
        assert_eq!(parsed.tokens.output, 9);
    }

    #[test]
    fn gzip_sse_decoder_handles_arbitrary_chunk_boundaries() {
        let encoded = gzip(SSE_USAGE_RESPONSE);
        for chunk_size in 1..=encoded.len().min(31) {
            let mut parser = parser_for(UsageContentEncoding::Gzip, true);
            for chunk in encoded.chunks(chunk_size) {
                parser.feed(chunk);
            }
            parser.finish(false);
            assert!(!parser.is_invalid(), "chunk={chunk_size}");
            assert!(parser.is_complete(), "chunk={chunk_size}");
            let parsed = parser.parsed().unwrap();
            assert_eq!(parsed.message_id.as_deref(), Some("msg_compressed_sse"));
            assert_eq!(parsed.tokens.input, 11);
            assert_eq!(parsed.tokens.output, 9);
            assert_eq!(parsed.tokens.cache_creation_5m, 7);
        }
    }

    #[test]
    fn compressed_sse_validates_trailer_at_eof_but_allows_client_drop_after_usage() {
        let mut encoded = gzip(SSE_USAGE_RESPONSE);
        let crc_offset = encoded.len() - 8;
        encoded[crc_offset] ^= 0xff;

        let mut eof_parser = parser_for(UsageContentEncoding::Gzip, true);
        eof_parser.feed(&encoded);
        eof_parser.finish(false);
        assert!(eof_parser.is_invalid());
        assert!(eof_parser.parsed().is_some());

        let encoded = incomplete_gzip_prefix(SSE_USAGE_RESPONSE);
        let mut dropped_parser = parser_for(UsageContentEncoding::Gzip, true);
        for chunk in encoded.chunks(7) {
            dropped_parser.feed(chunk);
        }
        assert!(dropped_parser.is_complete());
        dropped_parser.finish(true);
        assert!(!dropped_parser.is_invalid());
        assert_eq!(dropped_parser.parsed().unwrap().tokens.output, 9);
    }

    #[test]
    fn malformed_and_unsupported_encodings_fail_closed() {
        let mut malformed = parser_for(UsageContentEncoding::Gzip, false);
        malformed.feed(JSON_USAGE_RESPONSE);
        malformed.finish(false);
        assert!(malformed.is_invalid());
        assert!(malformed.parsed().is_none());

        let mut stacked = parser_for(UsageContentEncoding::Unsupported, false);
        stacked.feed(JSON_USAGE_RESPONSE);
        stacked.finish(false);
        assert!(stacked.is_invalid());
        assert!(stacked.parsed().is_none());
    }

    #[test]
    fn singapore_day_uses_fixed_utc_plus_eight() {
        let before_midnight = DateTime::parse_from_rfc3339("2026-08-11T15:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let after_midnight = DateTime::parse_from_rfc3339("2026-08-11T16:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            sg_day_from_utc(after_midnight),
            sg_day_from_utc(before_midnight) + 1
        );
    }

    #[test]
    fn week_buckets_start_on_monday_and_clip_to_query_range() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 12).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 8, 18).unwrap();
        let mut metrics = BTreeMap::new();
        metrics.insert(
            sg_day_from_date(start),
            UsageMetrics {
                request_count: 1,
                tokens: UsageTokens {
                    input: 7,
                    ..UsageTokens::default()
                },
                ..UsageMetrics::default()
            },
        );
        metrics.insert(
            sg_day_from_date(end),
            UsageMetrics {
                request_count: 2,
                tokens: UsageTokens {
                    output: 9,
                    ..UsageTokens::default()
                },
                ..UsageMetrics::default()
            },
        );

        let buckets = build_buckets(start, end, UsageGranularity::Week, &metrics);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].key, "2026-08-10");
        assert_eq!(buckets[0].start_date, start);
        assert_eq!(
            buckets[0].end_date,
            NaiveDate::from_ymd_opt(2026, 8, 16).unwrap()
        );
        assert_eq!(buckets[1].key, "2026-08-17");
        assert_eq!(buckets[1].end_date, end);
        assert_eq!(
            buckets[0].start_at_utc.to_rfc3339(),
            "2026-08-11T16:00:00+00:00"
        );
        assert_eq!(buckets[1].metrics.request_count, 2);
    }

    #[test]
    fn month_bucket_sums_daily_metrics_without_changing_total() {
        let start = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let mut metrics = BTreeMap::new();
        for (date, input) in [(start, 3), (end, 5)] {
            metrics.insert(
                sg_day_from_date(date),
                UsageMetrics {
                    request_count: 1,
                    tokens: UsageTokens {
                        input,
                        ..UsageTokens::default()
                    },
                    known_cost_nano_usd: input,
                    ..UsageMetrics::default()
                },
            );
        }
        let buckets = build_buckets(start, end, UsageGranularity::Month, &metrics);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].key, "2026-07-01");
        assert_eq!(buckets[1].key, "2026-08-01");
        assert_eq!(
            buckets
                .iter()
                .map(|bucket| bucket.metrics.request_count)
                .sum::<i64>(),
            2
        );
        assert_eq!(
            buckets
                .iter()
                .map(|bucket| bucket.metrics.tokens.input)
                .sum::<i64>(),
            8
        );
        assert_eq!(
            buckets
                .iter()
                .map(|bucket| bucket.metrics.known_cost_nano_usd)
                .sum::<i64>(),
            8
        );
    }

    #[tokio::test]
    async fn historical_backfill_prices_known_models_and_keeps_unknown_models() {
        sqlx::any::install_default_drivers();
        let path = std::env::temp_dir().join(format!(
            "ccbridge_pricing_backfill_{}.db",
            rand::random::<u64>()
        ));
        let pool = crate::store::db::init_db("sqlite", path.to_str().unwrap())
            .await
            .unwrap();
        crate::store::db::migrate(&pool, "sqlite").await.unwrap();
        let store = UsageStore::new(pool.clone(), "sqlite".into());
        let tokens = UsageTokens {
            input: 289,
            output: 2_027,
            cache_creation_5m: 6_099,
            cache_creation_1h: 0,
            cache_read: 4_272_647,
        };
        let unpriced = UsageCosts {
            input_nano_usd: None,
            output_nano_usd: None,
            cache_creation_5m_nano_usd: None,
            cache_creation_1h_nano_usd: Some(0),
            cache_read_nano_usd: None,
            known_nano_usd: 0,
            complete: false,
            unpriced_tokens: tokens.total(),
        };
        let event = |key: &str, model: &str| UsageEvent {
            dedup_key: key.into(),
            upstream_message_id: None,
            upstream_request_id: None,
            occurred_at_utc: Utc::now(),
            sg_day: sg_day_from_utc(Utc::now()),
            account_id: 1,
            api_token_id: 1,
            model: model.into(),
            tokens: tokens.clone(),
            costs: unpriced.clone(),
            pricing_version: "old-missing".into(),
            pricing_model_key: None,
            http_status: 200,
            is_stream: true,
        };
        store
            .insert_batch(&[
                event("known", "claude-opus-5"),
                event("unknown", "still-unknown"),
            ])
            .await
            .unwrap();

        let updated =
            backfill_unpriced_usage(&store, &PricingEngine::from_override_json(None).unwrap())
                .await
                .unwrap();
        assert_eq!(updated, 1);
        let known: (String, i32, Option<String>) = sqlx::query_as(
            "SELECT CAST(known_cost_nano_usd AS TEXT), cost_complete, pricing_model_key \
             FROM usage_events WHERE dedup_key = 'known'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(known.0.parse::<i64>().unwrap(), 2_226_562_250);
        assert_eq!(known.1, 1);
        assert_eq!(known.2.as_deref(), Some("claude-opus-5"));
        let unknown: i32 = sqlx::query_scalar(
            "SELECT cost_complete FROM usage_events WHERE dedup_key = 'unknown'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unknown, 0);

        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn observed_stream_preserves_bytes_and_chunk_boundaries() {
        sqlx::any::install_default_drivers();
        let path =
            std::env::temp_dir().join(format!("ccbridge_observer_{}.db", rand::random::<u64>()));
        let pool = crate::store::db::init_db("sqlite", path.to_str().unwrap())
            .await
            .unwrap();
        crate::store::db::migrate(&pool, "sqlite").await.unwrap();
        let store = Arc::new(UsageStore::new(pool.clone(), "sqlite".into()));
        let service = UsageService::start_without_pricing_refresh(
            store,
            PricingEngine::from_override_json(None).unwrap(),
        )
        .await;
        let chunks = vec![
            Bytes::from_static(b"{\"id\":\"msg_1\","),
            Bytes::from_static(b"\"model\":\"claude-sonnet-4-6\","),
            Bytes::from_static(b"\"usage\":{\"input_tokens\":1}}"),
        ];
        let stream =
            futures_util::stream::iter(chunks.clone().into_iter().map(Ok::<_, std::io::Error>));
        let mut attempt = UsageAttempt::begin(1, 2, "fallback".into(), false);
        attempt.complete_response(200, Some("req_1".into()), UsageContentEncoding::Identity);
        let actual: Vec<Bytes> = service
            .observe_stream(stream, attempt)
            .map(|item| item.unwrap())
            .collect()
            .await;
        assert_eq!(actual, chunks);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let count = loop {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM usage_events")
                .fetch_one(&pool)
                .await
                .unwrap();
            if count == 1 || tokio::time::Instant::now() >= deadline {
                break count;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(count, 1);
        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn gzip_observed_stream_preserves_compressed_chunks_and_persists_usage() {
        sqlx::any::install_default_drivers();
        let path = std::env::temp_dir().join(format!(
            "ccbridge_compressed_observer_{}.db",
            rand::random::<u64>()
        ));
        let pool = crate::store::db::init_db("sqlite", path.to_str().unwrap())
            .await
            .unwrap();
        crate::store::db::migrate(&pool, "sqlite").await.unwrap();
        let store = Arc::new(UsageStore::new(pool.clone(), "sqlite".into()));
        let service = UsageService::start_without_pricing_refresh(
            store,
            PricingEngine::from_override_json(None).unwrap(),
        )
        .await;
        let encoded = gzip(JSON_USAGE_RESPONSE);
        let chunks: Vec<Bytes> = encoded.chunks(7).map(Bytes::copy_from_slice).collect();
        let stream =
            futures_util::stream::iter(chunks.clone().into_iter().map(Ok::<_, std::io::Error>));
        let mut attempt = UsageAttempt::begin(1, 2, "fallback".into(), false);
        attempt.complete_response(
            200,
            Some("req_compressed".into()),
            UsageContentEncoding::Gzip,
        );
        let actual: Vec<Bytes> = service
            .observe_stream(stream, attempt)
            .map(|item| item.unwrap())
            .collect()
            .await;
        assert_eq!(actual, chunks);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let row = loop {
            let row = sqlx::query_as::<_, (i64, i64, i64)>(
                "SELECT input_tokens, output_tokens, cache_creation_5m_tokens FROM usage_events",
            )
            .fetch_optional(&pool)
            .await
            .unwrap();
            if row.is_some() || tokio::time::Instant::now() >= deadline {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(row, Some((11, 9, 7)));

        let failures_before = service.health().parse_failed_total;
        let invalid_chunks = vec![Bytes::from_static(b"not gzip")];
        let stream = futures_util::stream::iter(
            invalid_chunks
                .clone()
                .into_iter()
                .map(Ok::<_, std::io::Error>),
        );
        let mut attempt = UsageAttempt::begin(1, 2, "fallback".into(), false);
        attempt.complete_response(200, None, UsageContentEncoding::Gzip);
        let actual: Vec<Bytes> = service
            .observe_stream(stream, attempt)
            .map(|item| item.unwrap())
            .collect()
            .await;
        assert_eq!(actual, invalid_chunks);
        assert_eq!(service.health().parse_failed_total, failures_before + 1);

        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn compressed_sse_client_drop_persists_observed_usage_without_message_stop() {
        sqlx::any::install_default_drivers();
        let path = std::env::temp_dir().join(format!(
            "ccbridge_compressed_sse_drop_{}.db",
            rand::random::<u64>()
        ));
        let pool = crate::store::db::init_db("sqlite", path.to_str().unwrap())
            .await
            .unwrap();
        crate::store::db::migrate(&pool, "sqlite").await.unwrap();
        let store = Arc::new(UsageStore::new(pool.clone(), "sqlite".into()));
        let service = UsageService::start_without_pricing_refresh(
            store,
            PricingEngine::from_override_json(None).unwrap(),
        )
        .await;
        let sse = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_drop\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":11}}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n"
        );
        let encoded = incomplete_gzip_prefix(sse.as_bytes());
        let chunks: Vec<Bytes> = encoded.chunks(7).map(Bytes::copy_from_slice).collect();
        let chunk_count = chunks.len();
        let stream = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>))
            .chain(futures_util::stream::pending());
        let mut attempt = UsageAttempt::begin(1, 2, "fallback".into(), true);
        attempt.complete_response(200, None, UsageContentEncoding::Gzip);
        let mut observed = Box::pin(service.observe_stream(stream, attempt));
        for _ in 0..chunk_count {
            assert!(observed.next().await.is_some());
        }
        drop(observed);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let row = loop {
            let row = sqlx::query_as::<_, (i64, i64)>(
                "SELECT input_tokens, output_tokens FROM usage_events",
            )
            .fetch_optional(&pool)
            .await
            .unwrap();
            if row.is_some() || tokio::time::Instant::now() >= deadline {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(row, Some((11, 9)));

        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
