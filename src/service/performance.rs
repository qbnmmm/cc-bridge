use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::response::Response;
use chrono::{DateTime, Utc};
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::model::performance::*;
use crate::store::performance_store::{PerformanceStore, sg_day};

const QUEUE_SIZE: usize = 4096;
const ACTIVE_LIMIT: usize = 10_000;

#[derive(Default)]
struct Counters {
    persisted: AtomicU64,
    dropped: AtomicU64,
    write_failed: AtomicU64,
    active_dropped: AtomicU64,
    parse_failed: AtomicU64,
    expired: AtomicU64,
}

#[derive(Serialize)]
pub struct PerformanceHealth {
    pub enabled: bool,
    pub instance_id: String,
    pub since_utc: DateTime<Utc>,
    pub queue_depth: usize,
    pub persisted_total: u64,
    pub dropped_total: u64,
    pub write_failed_total: u64,
    pub active_tracking_dropped: u64,
    pub parse_failed_total: u64,
    pub retention_expired_total: u64,
}

#[derive(Serialize)]
pub struct ActivePerformance {
    pub scope: &'static str,
    pub instance_id: String,
    pub since_utc: DateTime<Utc>,
    pub generated_at_utc: DateTime<Utc>,
    pub health: PerformanceHealth,
    #[serde(flatten)]
    pub page: PerformancePage<PerformanceEvent>,
}

pub struct PerformanceService {
    pub store: Arc<PerformanceStore>,
    enabled: bool,
    instance_id: String,
    since: DateTime<Utc>,
    active: Mutex<HashMap<String, PerformanceHandle>>,
    sender: mpsc::Sender<PerformanceEvent>,
    counters: Arc<Counters>,
}

#[derive(Clone)]
pub struct PerformanceHandle(Arc<Mutex<RequestState>>);

struct RequestState {
    start: Instant,
    event: PerformanceEvent,
    last_content: Option<i64>,
    batch: u64,
    last_content_batch: Option<u64>,
    content_batches: u64,
    finished: bool,
}

pub struct PerformanceGuard {
    service: Arc<PerformanceService>,
    pub handle: PerformanceHandle,
    finished: bool,
}

impl PerformanceService {
    pub fn start(store: Arc<PerformanceStore>, enabled: bool) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(QUEUE_SIZE);
        let counters = Arc::new(Counters::default());
        tokio::spawn(run_writer(store.clone(), receiver, counters.clone()));
        let cleanup = store.clone();
        tokio::spawn(async move {
            loop {
                let min_day = retention_day();
                loop {
                    match cleanup.delete_before(min_day).await {
                        Ok(1000) => tokio::time::sleep(Duration::from_millis(50)).await,
                        Ok(_) => break,
                        Err(_) => {
                            tracing::warn!("performance retention cleanup failed");
                            break;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
        Arc::new(Self {
            store,
            enabled,
            instance_id: uuid::Uuid::new_v4().to_string(),
            since: Utc::now(),
            active: Mutex::new(HashMap::new()),
            sender,
            counters,
        })
    }

    pub fn begin(self: &Arc<Self>) -> Option<PerformanceGuard> {
        if !self.enabled {
            return None;
        }
        let event = PerformanceEvent {
            request_id: uuid::Uuid::new_v4().to_string(),
            instance_id: self.instance_id.clone(),
            started_at_utc: Utc::now(),
            completed_at_utc: None,
            account_id: None,
            api_token_id: None,
            request_model: None,
            response_model: None,
            upstream_request_id: None,
            is_stream: false,
            upstream_status: None,
            downstream_status: None,
            outcome: None,
            observation_quality: "observed".into(),
            stop_reason: None,
            phase: "preparing".into(),
            stages_ms: BTreeMap::new(),
            first_byte_ms: None,
            first_content_ms: None,
            first_text_ms: None,
            model_completed_ms: None,
            duration_ms: None,
            age_ms: 0,
            content_idle_ms: 0,
            max_content_gap_ms: None,
            output_tokens: None,
            output_tokens_per_second: None,
        };
        let handle = PerformanceHandle(Arc::new(Mutex::new(RequestState {
            start: Instant::now(),
            event,
            last_content: None,
            batch: 0,
            last_content_batch: None,
            content_batches: 0,
            finished: false,
        })));
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.len() < ACTIVE_LIMIT {
            let id = handle.snapshot().request_id;
            active.insert(id, handle.clone());
        } else {
            self.counters.active_dropped.fetch_add(1, Ordering::Relaxed);
        }
        Some(PerformanceGuard {
            service: self.clone(),
            handle,
            finished: false,
        })
    }

    pub fn health(&self) -> PerformanceHealth {
        PerformanceHealth {
            enabled: self.enabled,
            instance_id: self.instance_id.clone(),
            since_utc: self.since,
            queue_depth: QUEUE_SIZE - self.sender.capacity(),
            persisted_total: self.counters.persisted.load(Ordering::Relaxed),
            dropped_total: self.counters.dropped.load(Ordering::Relaxed),
            write_failed_total: self.counters.write_failed.load(Ordering::Relaxed),
            active_tracking_dropped: self.counters.active_dropped.load(Ordering::Relaxed),
            parse_failed_total: self.counters.parse_failed.load(Ordering::Relaxed),
            retention_expired_total: self.counters.expired.load(Ordering::Relaxed),
        }
    }

    pub fn active(&self, f: &PerformanceFilter) -> ActivePerformance {
        let handles: Vec<_> = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        let mut items: Vec<_> = handles
            .into_iter()
            .map(|h| h.snapshot())
            .filter(|e| {
                f.account_id.is_none_or(|id| e.account_id == Some(id))
                    && f.api_token_id.is_none_or(|id| e.api_token_id == Some(id))
                    && f.model
                        .as_ref()
                        .is_none_or(|m| e.request_model.as_ref() == Some(m))
                    && f.instance_id.as_ref().is_none_or(|id| &e.instance_id == id)
                    && f.is_stream.is_none_or(|v| e.is_stream == v)
            })
            .collect();
        items.sort_by(|a, b| {
            b.age_ms
                .cmp(&a.age_ms)
                .then(a.request_id.cmp(&b.request_id))
        });
        let total = items.len() as i64;
        let items = items
            .into_iter()
            .skip(((f.page - 1) * f.page_size) as usize)
            .take(f.page_size as usize)
            .collect();
        ActivePerformance {
            scope: "instance",
            instance_id: self.instance_id.clone(),
            since_utc: self.since,
            generated_at_utc: Utc::now(),
            health: self.health(),
            page: PerformancePage {
                items,
                total,
                page: f.page,
                page_size: f.page_size,
            },
        }
    }
}

impl PerformanceHandle {
    fn update(&self, f: impl FnOnce(&mut RequestState, i64)) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.finished {
            return;
        }
        let elapsed = millis(state.start.elapsed());
        f(&mut state, elapsed);
    }
    pub fn stage(&self, name: &'static str, duration: Duration) {
        self.update(|s, _| {
            s.event.stages_ms.insert(name.into(), millis(duration));
        });
    }
    pub fn phase(&self, phase: &'static str) {
        self.update(|s, _| s.event.phase = phase.into());
    }
    pub fn token(&self, id: i64) {
        self.update(|s, _| s.event.api_token_id = Some(id));
    }
    pub fn account(&self, id: i64) {
        self.update(|s, _| s.event.account_id = Some(id));
    }
    pub fn request(&self, model: Option<&str>, streaming: bool) {
        self.update(|s, _| {
            s.event.request_model = model.map(safe_label);
            s.event.is_stream = streaming;
        });
    }
    pub fn upstream(&self, status: u16, request_id: Option<&str>, streaming: bool) {
        self.update(|s, _| {
            s.event.upstream_status = Some(status);
            s.event.upstream_request_id = request_id.map(safe_label);
            s.event.is_stream = streaming;
            s.event.phase = "waiting_content".into();
        });
    }
    pub fn failure(&self, outcome: PerformanceOutcome) {
        self.update(|s, _| {
            if s.event.outcome.is_none() {
                s.event.outcome = Some(outcome);
            }
        });
    }
    pub fn parse_failed(&self) {
        self.update(|s, _| s.event.observation_quality = "parse_failed".into());
    }
    pub fn bytes(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.update(|s, ms| {
            s.batch += 1;
            s.event.first_byte_ms.get_or_insert(ms);
        });
    }
    pub fn snapshot(&self) -> PerformanceEvent {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut e = state.event.clone();
        let elapsed = e
            .duration_ms
            .unwrap_or_else(|| millis(state.start.elapsed()));
        e.age_ms = elapsed;
        e.content_idle_ms = e
            .model_completed_ms
            .unwrap_or(elapsed)
            .saturating_sub(state.last_content.unwrap_or(0));
        if let Some(last) = state.last_content {
            e.max_content_gap_ms = Some(
                e.max_content_gap_ms
                    .unwrap_or(0)
                    .max(e.model_completed_ms.unwrap_or(elapsed).saturating_sub(last)),
            );
        }
        e
    }

    pub fn observe_json(&self, value: &Value, streaming: bool) {
        self.update(|s, ms| {
            let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
            let message = if kind == "message_start" {
                value.get("message").unwrap_or(value)
            } else {
                value
            };
            if let Some(model) = message.get("model").and_then(Value::as_str) {
                s.event.response_model = Some(safe_label(model));
            }
            if let Some(tokens) = message
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(Value::as_i64)
                .filter(|n| *n >= 0)
            {
                s.event.output_tokens = Some(tokens);
            }
            if let Some(reason) = message
                .get("stop_reason")
                .or_else(|| value.get("delta").and_then(|d| d.get("stop_reason")))
                .and_then(Value::as_str)
            {
                s.event.stop_reason = Some(safe_label(reason));
            }
            if kind == "error" {
                s.event.outcome = Some(PerformanceOutcome::StreamError);
            }
            if streaming {
                let content = match kind {
                    "content_block_delta" => value.get("delta"),
                    "content_block_start" => value.get("content_block"),
                    _ => None,
                };
                if let Some(content) = content {
                    let content_type = content.get("type").and_then(Value::as_str).unwrap_or("");
                    let phase = match content_type {
                        "text" | "text_delta" if nonempty(content.get("text")) => Some("text"),
                        "thinking" | "thinking_delta" if nonempty(content.get("thinking")) => {
                            Some("thinking")
                        }
                        "input_json_delta" if nonempty(content.get("partial_json")) => Some("tool"),
                        "tool_use"
                            if content
                                .get("input")
                                .and_then(Value::as_object)
                                .is_some_and(|o| !o.is_empty()) =>
                        {
                            Some("tool")
                        }
                        "text" | "text_delta" | "thinking" | "thinking_delta"
                        | "input_json_delta" | "tool_use" | "signature_delta" => None,
                        _ => {
                            s.event.observation_quality = "partial".into();
                            None
                        }
                    };
                    if let Some(phase) = phase {
                        record_content(s, ms, phase);
                    }
                }
                if kind == "message_stop" {
                    s.event.model_completed_ms = Some(ms);
                    s.event.phase = "finishing".into();
                }
            } else if kind == "message" && value.get("content").is_some_and(Value::is_array) {
                // Buffered JSON reveals completion, not when its tokens were generated.
                s.event.model_completed_ms = Some(ms);
                s.event.phase = "finishing".into();
            }
        });
    }
}

fn nonempty(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}
fn record_content(s: &mut RequestState, ms: i64, phase: &str) {
    if let Some(last) = s.last_content {
        s.event.max_content_gap_ms = Some(s.event.max_content_gap_ms.unwrap_or(0).max(ms - last));
    }
    s.last_content = Some(ms);
    s.event.first_content_ms.get_or_insert(ms);
    if phase == "text" {
        s.event.first_text_ms.get_or_insert(ms);
    }
    s.event.phase = phase.into();
    if s.last_content_batch != Some(s.batch) {
        s.content_batches += 1;
        s.last_content_batch = Some(s.batch);
    }
}

impl PerformanceGuard {
    pub fn response(self, response: Response) -> Response {
        self.handle.update(|s, _| {
            let status = response.status().as_u16();
            s.event.downstream_status = Some(status);
            if !(200..300).contains(&status) && s.event.outcome.is_none() {
                s.event.outcome = Some(if s.event.upstream_status.is_some() {
                    PerformanceOutcome::HttpError
                } else {
                    PerformanceOutcome::LocalError
                });
            }
        });
        response.map(|body| {
            let mut guard = self;
            if body.is_end_stream() {
                guard.finish(false);
                body
            } else {
                Body::new(PerformanceBody {
                    inner: Box::pin(body),
                    guard: Some(guard),
                })
            }
        })
    }

    fn finish(&mut self, aborted: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        let mut event = self.handle.snapshot();
        {
            let mut state = self.handle.0.lock().unwrap_or_else(|e| e.into_inner());
            state.finished = true;
            event.duration_ms = Some(event.age_ms);
            event.completed_at_utc = Some(Utc::now());
            let outcome = event.outcome.unwrap_or_else(|| {
                if aborted {
                    PerformanceOutcome::Aborted
                } else if event.observation_quality == "parse_failed" {
                    PerformanceOutcome::Unknown
                } else if event.model_completed_ms.is_some() {
                    PerformanceOutcome::Success
                } else if event.is_stream {
                    PerformanceOutcome::Incomplete
                } else {
                    PerformanceOutcome::Unknown
                }
            });
            event.outcome = Some(outcome);
            if outcome == PerformanceOutcome::Success
                && event.observation_quality == "observed"
                && event.is_stream
                && state.content_batches > 1
            {
                if let (Some(tokens), Some(first), Some(end)) = (
                    event.output_tokens,
                    event.first_content_ms,
                    event.model_completed_ms,
                ) {
                    if end > first {
                        event.output_tokens_per_second =
                            Some(tokens as f64 * 1000.0 / (end - first) as f64);
                    }
                }
            }
            state.event = event.clone();
        }
        self.service
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&event.request_id);
        if event.observation_quality == "parse_failed" {
            self.service
                .counters
                .parse_failed
                .fetch_add(1, Ordering::Relaxed);
        }
        if sg_day(event.started_at_utc.timestamp_millis()) < retention_day() {
            self.service
                .counters
                .expired
                .fetch_add(1, Ordering::Relaxed);
        } else if self.service.sender.try_send(event).is_err() {
            let count = self
                .service
                .counters
                .dropped
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            if count == 1 || count % 100 == 0 {
                tracing::warn!(count, "performance completion queue unavailable");
            }
        }
    }
}
impl Drop for PerformanceGuard {
    fn drop(&mut self) {
        self.finish(true);
    }
}

struct PerformanceBody {
    inner: Pin<Box<Body>>,
    guard: Option<PerformanceGuard>,
}
impl HttpBody for PerformanceBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        let result = this.inner.as_mut().poll_frame(cx);
        match &result {
            Poll::Ready(Some(Err(_))) => {
                if let Some(mut guard) = this.guard.take() {
                    guard.handle.failure(PerformanceOutcome::StreamError);
                    guard.finish(false);
                }
            }
            Poll::Ready(None) => {
                if let Some(mut guard) = this.guard.take() {
                    guard.finish(false);
                }
            }
            Poll::Ready(Some(Ok(_))) if this.inner.is_end_stream() => {
                if let Some(mut guard) = this.guard.take() {
                    guard.finish(false);
                }
            }
            _ => {}
        }
        result
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
}

pub fn retention_day() -> i32 {
    sg_day(Utc::now().timestamp_millis()) - 29
}
fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}
fn safe_label(v: &str) -> String {
    if !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/[]".contains(&b))
    {
        v.to_owned()
    } else {
        "other".into()
    }
}

async fn run_writer(
    store: Arc<PerformanceStore>,
    mut receiver: mpsc::Receiver<PerformanceEvent>,
    counters: Arc<Counters>,
) {
    while let Some(first) = receiver.recv().await {
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        while batch.len() < 128 {
            match tokio::time::timeout_at(deadline, receiver.recv()).await {
                Ok(Some(e)) => batch.push(e),
                _ => break,
            }
        }
        let before = batch.len();
        let min_day = retention_day();
        batch.retain(|e| sg_day(e.started_at_utc.timestamp_millis()) >= min_day);
        counters
            .expired
            .fetch_add((before - batch.len()) as u64, Ordering::Relaxed);
        if batch.is_empty() {
            continue;
        }
        for retry in 0..3 {
            match store.insert_batch(&batch).await {
                Ok(n) => {
                    counters.persisted.fetch_add(n, Ordering::Relaxed);
                    break;
                }
                Err(_) if retry < 2 => {
                    tokio::time::sleep(Duration::from_millis(50 * (1 << retry))).await
                }
                Err(_) => {
                    counters
                        .write_failed
                        .fetch_add(batch.len() as u64, Ordering::Relaxed);
                    let total = counters
                        .dropped
                        .fetch_add(batch.len() as u64, Ordering::Relaxed)
                        + batch.len() as u64;
                    if total == batch.len() as u64
                        || total / 100 != (total - batch.len() as u64) / 100
                    {
                        tracing::warn!(total, "performance batch write failed");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn test_service() -> (Arc<PerformanceService>, mpsc::Receiver<PerformanceEvent>) {
    sqlx::any::install_default_drivers();
    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(1)
        .connect_lazy("sqlite::memory:")
        .unwrap();
    let (sender, receiver) = mpsc::channel(2);
    (
        Arc::new(PerformanceService {
            store: Arc::new(PerformanceStore::new(pool)),
            enabled: true,
            instance_id: "test-instance".into(),
            since: Utc::now(),
            active: Mutex::new(HashMap::new()),
            sender,
            counters: Arc::new(Counters::default()),
        }),
        receiver,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;

    fn advance(h: &PerformanceHandle, ms: u64) {
        h.0.lock().unwrap().start -= Duration::from_millis(ms);
    }

    #[tokio::test]
    async fn heartbeats_do_not_hide_thirty_minute_wait_then_thinking_and_text_are_distinct() {
        let (svc, mut rx) = test_service();
        let mut g = svc.begin().unwrap();
        let h = g.handle.clone();
        h.request(Some("claude-opus"), true);
        h.bytes(b"data: ping\n\n");
        h.observe_json(
            &json!({"type":"message_start","message":{"model":"claude-opus"}}),
            true,
        );
        advance(&h, 1_801_000);
        h.bytes(b"ping");
        h.observe_json(&json!({"type":"ping"}), true);
        let e = h.snapshot();
        assert!(e.age_ms >= 1_801_000 && e.content_idle_ms >= 1_801_000);
        assert!(e.first_byte_ms.is_some());
        assert_eq!(e.first_content_ms, None);
        assert_eq!(svc.active.lock().unwrap().len(), 1);
        h.observe_json(&json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"private reasoning"}}), true);
        assert_eq!(h.snapshot().first_text_ms, None);
        advance(&h, 1000);
        h.bytes(b"text");
        h.observe_json(&json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"private answer"}}), true);
        advance(&h, 1000);
        h.observe_json(&json!({"type":"message_delta","usage":{"output_tokens":100},"delta":{"stop_reason":"end_turn"}}), true);
        h.observe_json(&json!({"type":"message_stop"}), true);
        g.finish(false);
        let e = rx.try_recv().unwrap();
        assert_eq!(e.outcome, Some(PerformanceOutcome::Success));
        assert!(e.first_text_ms.unwrap() > e.first_content_ms.unwrap());
        assert!((e.output_tokens_per_second.unwrap() - 50.0).abs() < 1.0);
        assert!(e.max_content_gap_ms.unwrap() >= 1000);
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("private reasoning") && !json.contains("private answer"));
        assert!(svc.active.lock().unwrap().is_empty());
        drop(g);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn tool_only_success_and_single_chunk_have_no_fake_text_or_speed() {
        let (svc, mut rx) = test_service();
        let mut g = svc.begin().unwrap();
        let h = &g.handle;
        h.request(Some("test"), true);
        h.bytes(b"all events");
        h.observe_json(&json!({"type":"content_block_start","content_block":{"type":"tool_use","id":"x","name":"tool","input":{}}}), true);
        assert_eq!(h.snapshot().first_content_ms, None);
        h.observe_json(&json!({"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\"x\":1}"}}), true);
        h.observe_json(&json!({"type":"message_delta","usage":{"output_tokens":7},"delta":{"stop_reason":"tool_use"}}), true);
        h.observe_json(&json!({"type":"message_stop"}), true);
        g.finish(false);
        let e = rx.try_recv().unwrap();
        assert_eq!(e.outcome, Some(PerformanceOutcome::Success));
        assert!(e.first_content_ms.is_some());
        assert_eq!(e.first_text_ms, None);
        assert_eq!(e.output_tokens_per_second, None);
    }

    #[tokio::test]
    async fn errors_incomplete_parse_failures_and_abort_are_distinct() {
        for (outcome, aborted, complete, invalid, expected) in [
            (
                Some(PerformanceOutcome::ReadTimeout),
                false,
                false,
                false,
                PerformanceOutcome::ReadTimeout,
            ),
            (
                Some(PerformanceOutcome::StreamError),
                false,
                true,
                false,
                PerformanceOutcome::StreamError,
            ),
            (None, false, false, false, PerformanceOutcome::Incomplete),
            (None, false, true, true, PerformanceOutcome::Unknown),
            (None, true, true, false, PerformanceOutcome::Aborted),
        ] {
            let (svc, mut rx) = test_service();
            let mut g = svc.begin().unwrap();
            g.handle.request(None, true);
            if let Some(o) = outcome {
                g.handle.failure(o);
            }
            if complete {
                g.handle.observe_json(&json!({"type":"message_stop"}), true);
            }
            if invalid {
                g.handle.parse_failed();
            }
            g.finish(aborted);
            assert_eq!(rx.try_recv().unwrap().outcome, Some(expected));
        }
    }

    #[tokio::test]
    async fn response_is_active_until_body_finishes_or_drops_and_errors_keep_status() {
        let (svc, mut rx) = test_service();
        let g = svc.begin().unwrap();
        g.handle.upstream(429, None, true);
        let response = g.response(
            Response::builder()
                .status(429)
                .body(Body::from("safe error"))
                .unwrap(),
        );
        assert_eq!(svc.active.lock().unwrap().len(), 1);
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"safe error");
        assert_eq!(
            rx.try_recv().unwrap().outcome,
            Some(PerformanceOutcome::HttpError)
        );
        let g = svc.begin().unwrap();
        let stream = futures_util::stream::once(async {
            Ok::<_, std::io::Error>(Bytes::from_static(b"hello"))
        })
        .chain(futures_util::stream::pending());
        let response = g.response(Response::new(Body::from_stream(stream)));
        let mut body = response.into_body().into_data_stream();
        assert_eq!(body.next().await.unwrap().unwrap(), "hello");
        assert_eq!(svc.active.lock().unwrap().len(), 1);
        drop(body);
        assert_eq!(
            rx.try_recv().unwrap().outcome,
            Some(PerformanceOutcome::Aborted)
        );
    }

    #[tokio::test]
    async fn cancelling_handler_future_cleans_up_and_queue_pressure_never_blocks() {
        let (svc, mut rx) = test_service();
        let (tx, ready) = tokio::sync::oneshot::channel();
        let inner = svc.clone();
        let task = tokio::spawn(async move {
            let _g = inner.begin().unwrap();
            tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(
            rx.try_recv().unwrap().outcome,
            Some(PerformanceOutcome::Aborted)
        );
        for _ in 0..5 {
            drop(svc.begin());
        }
        assert_eq!(svc.health().dropped_total, 3);
        assert!(svc.active.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn body_wrapper_preserves_size_headers_and_trailers() {
        use http_body_util::{BodyExt, StreamBody};
        let (svc, mut rx) = test_service();
        let guard = svc.begin().unwrap();
        let response = guard.response(
            Response::builder()
                .header("x-test", "preserved")
                .body(Body::from("hello"))
                .unwrap(),
        );
        assert_eq!(response.headers()["x-test"], "preserved");
        assert_eq!(response.body().size_hint().exact(), Some(5));
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "hello"
        );
        rx.try_recv().unwrap();
        let mut trailers = axum::http::HeaderMap::new();
        trailers.insert("x-trailer", "kept".parse().unwrap());
        let frames = futures_util::stream::iter(vec![
            Ok::<_, std::io::Error>(Frame::data(Bytes::from_static(b"data"))),
            Ok(Frame::trailers(trailers)),
        ]);
        let guard = svc.begin().unwrap();
        let response = guard.response(Response::new(Body::new(StreamBody::new(frames))));
        let collected = response.into_body().collect().await.unwrap();
        assert_eq!(collected.trailers().unwrap()["x-trailer"], "kept");
        assert_eq!(collected.to_bytes(), "data");
        rx.try_recv().unwrap();
        assert!(svc.active.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn database_failure_drops_observation_without_affecting_response() {
        sqlx::any::install_default_drivers();
        let pool = sqlx::any::AnyPoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        pool.close().await;
        let svc = PerformanceService::start(Arc::new(PerformanceStore::new(pool)), true);
        let response = svc
            .begin()
            .unwrap()
            .response(Response::new(Body::from("still delivered")));
        let bytes = axum::body::to_bytes(response.into_body(), 100)
            .await
            .unwrap();
        assert_eq!(bytes, "still delivered");
        tokio::time::timeout(Duration::from_secs(3), async {
            while svc.health().write_failed_total == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(svc.health().dropped_total, 1);
        assert!(svc.active.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn disabled_collection_and_expired_completion_do_not_write_events() {
        let (mut svc, mut rx) = test_service();
        Arc::get_mut(&mut svc).unwrap().enabled = false;
        assert!(svc.begin().is_none());
        assert!(!svc.health().enabled);
        Arc::get_mut(&mut svc).unwrap().enabled = true;
        let guard = svc.begin().unwrap();
        guard
            .handle
            .update(|s, _| s.event.started_at_utc -= chrono::Duration::days(31));
        drop(guard);
        assert_eq!(svc.health().retention_expired_total, 1);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn non_streaming_json_no_usage_is_success_without_inventing_first_token() {
        let (svc, mut rx) = test_service();
        let mut g = svc.begin().unwrap();
        g.handle.bytes(b"json");
        g.handle.observe_json(
            &json!({"type":"message","content":[{"type":"text","text":"hi"}]}),
            false,
        );
        g.finish(false);
        let e = rx.try_recv().unwrap();
        assert_eq!(e.outcome, Some(PerformanceOutcome::Success));
        assert_eq!(e.first_content_ms, None);
        assert_eq!(e.first_text_ms, None);
        assert_eq!(e.output_tokens, None);
    }
}
