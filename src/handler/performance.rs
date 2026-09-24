use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::router::AppState;
use crate::error::AppError;
use crate::model::performance::*;
use crate::service::performance::{ActivePerformance, PerformanceHealth, retention_day};

#[derive(Default, Deserialize)]
pub struct PerformanceQuery {
    start_at: Option<DateTime<Utc>>,
    end_at: Option<DateTime<Utc>>,
    account_id: Option<i64>,
    api_token_id: Option<i64>,
    model: Option<String>,
    instance_id: Option<String>,
    outcome: Option<PerformanceOutcome>,
    is_stream: Option<bool>,
    min_duration_ms: Option<i64>,
    sort: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}
impl PerformanceQuery {
    fn validate(self) -> Result<PerformanceFilter, AppError> {
        let now = Utc::now().timestamp_millis();
        let end_ms = self.end_at.map(|v| v.timestamp_millis()).unwrap_or(now);
        let min_ms = i64::from(retention_day()) * 86_400_000 - 28_800_000;
        let start_ms = self
            .start_at
            .map(|v| v.timestamp_millis())
            .unwrap_or(end_ms - 3_600_000);
        if start_ms < min_ms || end_ms > now + 1000 || start_ms >= end_ms {
            return Err(AppError::BadRequest(
                "performance range must be within the last 30 Singapore calendar days".into(),
            ));
        }
        if self.account_id.is_some_and(|v| v <= 0)
            || self.api_token_id.is_some_and(|v| v <= 0)
            || self.min_duration_ms.is_some_and(|v| v < 0)
            || self.page.is_some_and(|v| !(1..=1_000_000).contains(&v))
            || self.page_size.is_some_and(|v| !(1..=100).contains(&v))
            || self.model.as_ref().is_some_and(|s| s.len() > 128)
            || self.instance_id.as_ref().is_some_and(|s| s.len() > 128)
        {
            return Err(AppError::BadRequest(
                "invalid performance filter or pagination".into(),
            ));
        }
        let duration_desc = match self.sort.as_deref().unwrap_or("duration_desc") {
            "duration_desc" => true,
            "created_at_desc" => false,
            _ => return Err(AppError::BadRequest("invalid performance sort".into())),
        };
        Ok(PerformanceFilter {
            start_ms,
            end_ms,
            account_id: self.account_id,
            api_token_id: self.api_token_id,
            model: self.model.filter(|s| !s.is_empty()),
            instance_id: self.instance_id.filter(|s| !s.is_empty()),
            outcome: self.outcome,
            is_stream: self.is_stream,
            min_duration_ms: self.min_duration_ms,
            duration_desc,
            page: self.page.unwrap_or(1),
            page_size: self.page_size.unwrap_or(20),
        })
    }
}

#[derive(Serialize)]
pub struct PerformanceOverview {
    #[serde(flatten)]
    report: PerformanceReport,
    health: PerformanceHealth,
}

pub async fn overview(
    State(s): State<AppState>,
    Query(q): Query<PerformanceQuery>,
) -> Result<Json<PerformanceOverview>, AppError> {
    let report = s.performance_svc.store.report(&q.validate()?).await?;
    Ok(Json(PerformanceOverview {
        report,
        health: s.performance_svc.health(),
    }))
}
pub async fn active(
    State(s): State<AppState>,
    Query(q): Query<PerformanceQuery>,
) -> Result<Json<ActivePerformance>, AppError> {
    Ok(Json(s.performance_svc.active(&q.validate()?)))
}
pub async fn requests(
    State(s): State<AppState>,
    Query(q): Query<PerformanceQuery>,
) -> Result<Json<PerformancePage<PerformanceEvent>>, AppError> {
    Ok(Json(s.performance_svc.store.list(&q.validate()?).await?))
}
pub async fn detail(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PerformanceEvent>, AppError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(AppError::BadRequest("invalid request id".into()));
    }
    Ok(Json(
        s.performance_svc.store.get(&id, retention_day()).await?,
    ))
}
pub async fn dimensions(
    State(s): State<AppState>,
) -> Result<Json<PerformanceDimensions>, AppError> {
    Ok(Json(
        s.performance_svc.store.dimensions(retention_day()).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_ranges_and_unsafe_filters() {
        for query in [
            PerformanceQuery {
                sort: Some("duration; DROP TABLE accounts".into()),
                ..Default::default()
            },
            PerformanceQuery {
                page: Some(i64::MAX),
                ..Default::default()
            },
            PerformanceQuery {
                min_duration_ms: Some(-1),
                ..Default::default()
            },
            PerformanceQuery {
                start_at: Some(Utc::now() - chrono::Duration::days(31)),
                ..Default::default()
            },
        ] {
            assert!(query.validate().is_err());
        }
        let q = PerformanceQuery::default().validate().unwrap();
        assert_eq!(q.end_ms - q.start_ms, 3_600_000);
    }
}
