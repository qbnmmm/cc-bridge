use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceOutcome {
    Success,
    LocalError,
    HttpError,
    SendError,
    SendTimeout,
    ReadTimeout,
    StreamError,
    Incomplete,
    Aborted,
    Unknown,
}

impl PerformanceOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::LocalError => "local_error",
            Self::HttpError => "http_error",
            Self::SendError => "send_error",
            Self::SendTimeout => "send_timeout",
            Self::ReadTimeout => "read_timeout",
            Self::StreamError => "stream_error",
            Self::Incomplete => "incomplete",
            Self::Aborted => "aborted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceEvent {
    pub request_id: String,
    pub instance_id: String,
    pub started_at_utc: DateTime<Utc>,
    pub completed_at_utc: Option<DateTime<Utc>>,
    pub account_id: Option<i64>,
    pub api_token_id: Option<i64>,
    pub request_model: Option<String>,
    pub response_model: Option<String>,
    pub upstream_request_id: Option<String>,
    pub is_stream: bool,
    pub upstream_status: Option<u16>,
    pub downstream_status: Option<u16>,
    pub outcome: Option<PerformanceOutcome>,
    pub observation_quality: String,
    pub stop_reason: Option<String>,
    pub phase: String,
    pub stages_ms: BTreeMap<String, i64>,
    pub first_byte_ms: Option<i64>,
    pub first_content_ms: Option<i64>,
    pub first_text_ms: Option<i64>,
    pub model_completed_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub age_ms: i64,
    pub content_idle_ms: i64,
    pub max_content_gap_ms: Option<i64>,
    pub output_tokens: Option<i64>,
    pub output_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformancePage<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone)]
pub struct PerformanceFilter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub account_id: Option<i64>,
    pub api_token_id: Option<i64>,
    pub model: Option<String>,
    pub instance_id: Option<String>,
    pub outcome: Option<PerformanceOutcome>,
    pub is_stream: Option<bool>,
    pub min_duration_ms: Option<i64>,
    pub duration_desc: bool,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PerformancePercentiles {
    pub sample_count: i64,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PerformanceCounts {
    pub completed_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub aborted_count: i64,
    pub unknown_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceBucket {
    pub start_ms: i64,
    #[serde(flatten)]
    pub counts: PerformanceCounts,
    pub duration: PerformancePercentiles,
    pub first_content: PerformancePercentiles,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceHistogram {
    pub label: String,
    pub duration_count: i64,
    pub first_content_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerformanceReport {
    pub start_ms: i64,
    pub end_ms: i64,
    pub bucket_ms: i64,
    pub counts: PerformanceCounts,
    pub duration: PerformancePercentiles,
    pub first_byte: PerformancePercentiles,
    pub first_content: PerformancePercentiles,
    pub first_text: PerformancePercentiles,
    pub output_speed: PerformancePercentiles,
    pub buckets: Vec<PerformanceBucket>,
    pub histogram: Vec<PerformanceHistogram>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PerformanceDimensions {
    pub accounts: Vec<crate::model::usage::UsageDimensionOption>,
    pub api_tokens: Vec<crate::model::usage::UsageDimensionOption>,
    pub models: Vec<String>,
    pub instances: Vec<String>,
}
