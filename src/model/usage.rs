use chrono::{DateTime, NaiveDate, Utc};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UsageTokens {
    pub input: i64,
    pub output: i64,
    pub cache_creation_5m: i64,
    pub cache_creation_1h: i64,
    pub cache_read: i64,
}

impl Serialize for UsageTokens {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("UsageTokens", 6)?;
        state.serialize_field("input", &self.input)?;
        state.serialize_field("output", &self.output)?;
        state.serialize_field("cache_creation_5m", &self.cache_creation_5m)?;
        state.serialize_field("cache_creation_1h", &self.cache_creation_1h)?;
        state.serialize_field("cache_read", &self.cache_read)?;
        state.serialize_field("total", &self.total())?;
        state.end()
    }
}

impl UsageTokens {
    pub fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_creation_5m)
            .saturating_add(self.cache_creation_1h)
            .saturating_add(self.cache_read)
    }

    pub fn has_usage(&self) -> bool {
        self.total() > 0
    }

    pub fn add_assign(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_creation_5m = self
            .cache_creation_5m
            .saturating_add(other.cache_creation_5m);
        self.cache_creation_1h = self
            .cache_creation_1h
            .saturating_add(other.cache_creation_1h);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageCosts {
    pub input_nano_usd: Option<i64>,
    pub output_nano_usd: Option<i64>,
    pub cache_creation_5m_nano_usd: Option<i64>,
    pub cache_creation_1h_nano_usd: Option<i64>,
    pub cache_read_nano_usd: Option<i64>,
    pub known_nano_usd: i64,
    pub complete: bool,
    pub unpriced_tokens: i64,
}

#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub dedup_key: String,
    pub upstream_message_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub occurred_at_utc: DateTime<Utc>,
    pub sg_day: i32,
    pub account_id: i64,
    pub api_token_id: i64,
    pub model: String,
    pub tokens: UsageTokens,
    pub costs: UsageCosts,
    pub pricing_version: String,
    pub pricing_model_key: Option<String>,
    pub http_status: u16,
    pub is_stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpricedUsageEvent {
    pub id: i64,
    pub model: String,
    pub tokens: UsageTokens,
}

#[derive(Debug, Clone)]
pub struct UsagePricingUpdate {
    pub id: i64,
    pub costs: UsageCosts,
    pub pricing_version: String,
    pub pricing_model_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageMetrics {
    pub request_count: i64,
    pub tokens: UsageTokens,
    #[serde(serialize_with = "serialize_i64_as_string")]
    pub known_cost_nano_usd: i64,
    pub cost_complete: bool,
    pub unpriced_request_count: i64,
    pub unpriced_tokens: i64,
}

impl Default for UsageMetrics {
    fn default() -> Self {
        Self {
            request_count: 0,
            tokens: UsageTokens::default(),
            known_cost_nano_usd: 0,
            cost_complete: true,
            unpriced_request_count: 0,
            unpriced_tokens: 0,
        }
    }
}

impl UsageMetrics {
    pub fn add_assign(&mut self, other: &Self) {
        self.request_count = self.request_count.saturating_add(other.request_count);
        self.tokens.add_assign(&other.tokens);
        self.known_cost_nano_usd = self
            .known_cost_nano_usd
            .saturating_add(other.known_cost_nano_usd);
        self.unpriced_request_count = self
            .unpriced_request_count
            .saturating_add(other.unpriced_request_count);
        self.unpriced_tokens = self.unpriced_tokens.saturating_add(other.unpriced_tokens);
        self.cost_complete = self.unpriced_request_count == 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyUsageRow {
    pub sg_day: i32,
    pub group_id: i64,
    pub group_text: String,
    pub metrics: UsageMetrics,
}

#[derive(Debug, Clone, Default)]
pub struct UsageFilters {
    pub start_sg_day: i32,
    pub end_sg_day: i32,
    pub account_id: Option<i64>,
    pub api_token_id: Option<i64>,
    pub model: Option<String>,
    pub group_by: UsageGroupBy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UsageGroupBy {
    Account,
    ApiToken,
    #[default]
    Model,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageGranularity {
    #[default]
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone)]
pub struct UsageReportQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub granularity: UsageGranularity,
    pub account_id: Option<i64>,
    pub api_token_id: Option<i64>,
    pub model: Option<String>,
    pub group_by: UsageGroupBy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageDateRange {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageBucket {
    pub key: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub start_at_utc: DateTime<Utc>,
    pub end_at_utc_exclusive: DateTime<Utc>,
    pub metrics: UsageMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageBreakdownRow {
    pub key: String,
    pub label: String,
    pub metrics: UsageMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageReport {
    pub timezone: &'static str,
    pub granularity: UsageGranularity,
    pub range: UsageDateRange,
    pub summary: UsageMetrics,
    pub buckets: Vec<UsageBucket>,
    pub breakdown: Vec<UsageBreakdownRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageDimensionOption {
    pub id: i64,
    pub label: String,
}

fn serialize_i64_as_string<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_addition_recomputes_completeness() {
        let mut total = UsageMetrics {
            cost_complete: true,
            ..UsageMetrics::default()
        };
        total.add_assign(&UsageMetrics {
            request_count: 1,
            tokens: UsageTokens {
                input: 3,
                ..UsageTokens::default()
            },
            known_cost_nano_usd: 9,
            cost_complete: false,
            unpriced_request_count: 1,
            unpriced_tokens: 3,
        });
        assert_eq!(total.tokens.total(), 3);
        assert!(!total.cost_complete);
    }

    #[test]
    fn token_serialization_includes_total() {
        let value = serde_json::to_value(UsageTokens {
            input: 1,
            output: 2,
            cache_creation_5m: 3,
            cache_creation_1h: 4,
            cache_read: 5,
        })
        .unwrap();
        assert_eq!(value["total"], 15);
    }
}
