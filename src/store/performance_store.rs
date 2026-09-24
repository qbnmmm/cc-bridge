use sqlx::{
    Any, AnyPool, Execute, QueryBuilder, Row,
    any::{AnyArguments, AnyRow},
};

use crate::error::AppError;
use crate::model::performance::*;
use crate::model::usage::UsageDimensionOption;

pub struct PerformanceStore {
    pool: AnyPool,
}

pub const PERFORMANCE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS request_performance_events (
    request_id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL,
    started_at_ms BIGINT NOT NULL,
    sg_day INTEGER NOT NULL,
    account_id BIGINT,
    api_token_id BIGINT,
    model TEXT,
    is_stream INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    duration_ms BIGINT,
    first_byte_ms BIGINT,
    first_content_ms BIGINT,
    first_text_ms BIGINT,
    output_speed DOUBLE PRECISION,
    event_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_performance_day ON request_performance_events(sg_day, started_at_ms);
CREATE INDEX IF NOT EXISTS idx_performance_account ON request_performance_events(account_id, sg_day);
CREATE INDEX IF NOT EXISTS idx_performance_token ON request_performance_events(api_token_id, sg_day);
CREATE INDEX IF NOT EXISTS idx_performance_model ON request_performance_events(model, sg_day);
"#;

impl PerformanceStore {
    pub fn new(pool: AnyPool) -> Self {
        Self { pool }
    }

    pub async fn insert_batch(&self, events: &[PerformanceEvent]) -> Result<u64, AppError> {
        let mut tx = self.pool.begin().await?;
        let mut inserted = 0;
        for e in events {
            let json = serde_json::to_string(e)
                .map_err(|err| AppError::Internal(format!("serialize performance: {err}")))?;
            // SQLx Any 0.7 erases NULL's type to INT4; cached PostgreSQL statements then
            // reject later BIGINT/TEXT/FLOAT8 values. Bind stable non-null types and
            // restore NULL in SQL (IDs/timings/rates are nonnegative, models nonempty).
            inserted += sqlx::query("INSERT INTO request_performance_events (request_id, instance_id, started_at_ms, sg_day, account_id, api_token_id, model, is_stream, outcome, duration_ms, first_byte_ms, first_content_ms, first_text_ms, output_speed, event_json) VALUES ($1,$2,$3,$4,NULLIF(CAST($5 AS BIGINT), -1),NULLIF(CAST($6 AS BIGINT), -1),NULLIF(CAST($7 AS TEXT), ''),$8,$9,NULLIF(CAST($10 AS BIGINT), -1),NULLIF(CAST($11 AS BIGINT), -1),NULLIF(CAST($12 AS BIGINT), -1),NULLIF(CAST($13 AS BIGINT), -1),NULLIF(CAST($14 AS DOUBLE PRECISION), -1),$15) ON CONFLICT (request_id) DO NOTHING")
                .bind(&e.request_id).bind(&e.instance_id).bind(e.started_at_utc.timestamp_millis())
                .bind(sg_day(e.started_at_utc.timestamp_millis()))
                .bind(e.account_id.unwrap_or(-1)).bind(e.api_token_id.unwrap_or(-1)).bind(e.request_model.as_deref().unwrap_or(""))
                .bind(i32::from(e.is_stream)).bind(e.outcome.unwrap_or(PerformanceOutcome::Unknown).as_str())
                .bind(e.duration_ms.unwrap_or(-1)).bind(e.first_byte_ms.unwrap_or(-1)).bind(e.first_content_ms.unwrap_or(-1)).bind(e.first_text_ms.unwrap_or(-1))
                .bind(e.output_tokens_per_second.unwrap_or(-1.0)).bind(json).execute(&mut *tx).await?.rows_affected();
        }
        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn list(
        &self,
        f: &PerformanceFilter,
    ) -> Result<PerformancePage<PerformanceEvent>, AppError> {
        let mut count = QueryBuilder::<Any>::new(
            "SELECT CAST(COUNT(*) AS BIGINT) FROM request_performance_events WHERE ",
        );
        where_filter(&mut count, f);
        let (sql, args) = numbered_query(&mut count);
        let total = sqlx::query_scalar_with::<Any, i64, _>(&sql, args)
            .fetch_one(&self.pool)
            .await?;
        let mut q =
            QueryBuilder::<Any>::new("SELECT event_json FROM request_performance_events WHERE ");
        where_filter(&mut q, f);
        q.push(if f.duration_desc {
            " ORDER BY duration_ms DESC, started_at_ms DESC, request_id"
        } else {
            " ORDER BY started_at_ms DESC, request_id"
        });
        q.push(" LIMIT ")
            .push_bind(f.page_size)
            .push(" OFFSET ")
            .push_bind((f.page - 1) * f.page_size);
        let (sql, args) = numbered_query(&mut q);
        let values: Vec<String> = sqlx::query_scalar_with(&sql, args)
            .fetch_all(&self.pool)
            .await?;
        let items = values
            .iter()
            .map(|v| decode_event(v))
            .collect::<Result<_, _>>()?;
        Ok(PerformancePage {
            items,
            total,
            page: f.page,
            page_size: f.page_size,
        })
    }

    pub async fn get(&self, id: &str, min_day: i32) -> Result<PerformanceEvent, AppError> {
        let json: String = sqlx::query_scalar("SELECT event_json FROM request_performance_events WHERE request_id=$1 AND sg_day >= $2")
            .bind(id).bind(min_day).fetch_one(&self.pool).await?;
        decode_event(&json)
    }

    pub async fn dimensions(&self, min_day: i32) -> Result<PerformanceDimensions, AppError> {
        let mut result = PerformanceDimensions::default();
        for (column, table, label, target) in [
            ("account_id", "accounts", "账号 #", &mut result.accounts),
            (
                "api_token_id",
                "api_tokens",
                "Token #",
                &mut result.api_tokens,
            ),
        ] {
            let sql = format!(
                "SELECT CAST(p.id AS BIGINT) AS id, COALESCE(NULLIF(a.name,''), '{label}' || CAST(p.id AS TEXT)) AS label FROM (SELECT DISTINCT {column} AS id FROM request_performance_events WHERE sg_day >= $1 AND {column} IS NOT NULL UNION SELECT id FROM {table}) p LEFT JOIN {table} a ON a.id=p.id ORDER BY label"
            );
            for row in sqlx::query(&sql)
                .bind(min_day)
                .fetch_all(&self.pool)
                .await?
            {
                target.push(UsageDimensionOption {
                    id: row.try_get("id")?,
                    label: row.try_get("label")?,
                });
            }
        }
        result.models = sqlx::query_scalar("SELECT DISTINCT model FROM request_performance_events WHERE sg_day >= $1 AND model IS NOT NULL ORDER BY model")
            .bind(min_day).fetch_all(&self.pool).await?;
        result.instances = sqlx::query_scalar("SELECT DISTINCT instance_id FROM request_performance_events WHERE sg_day >= $1 ORDER BY instance_id")
            .bind(min_day).fetch_all(&self.pool).await?;
        Ok(result)
    }

    pub async fn delete_before(&self, min_day: i32) -> Result<u64, AppError> {
        Ok(sqlx::query("DELETE FROM request_performance_events WHERE request_id IN (SELECT request_id FROM request_performance_events WHERE sg_day < $1 ORDER BY sg_day LIMIT 1000)")
            .bind(min_day).execute(&self.pool).await?.rows_affected())
    }

    pub async fn report(&self, f: &PerformanceFilter) -> Result<PerformanceReport, AppError> {
        let bucket_ms = [60_000, 300_000, 3_600_000, 86_400_000]
            .into_iter()
            .find(|size| (f.end_ms - f.start_ms + size - 1) / size <= 288)
            .unwrap_or(86_400_000);
        // All values are internal constants or validated integer timestamps, never SQL supplied by callers.
        let bucket_expr = format!(
            "CAST((started_at_ms - {}) / {bucket_ms} AS BIGINT)",
            f.start_ms
        );
        let mut q = QueryBuilder::<Any>::new(format!(
            "SELECT {bucket_expr} AS bucket, {} FROM request_performance_events WHERE ",
            counts_sql()
        ));
        where_filter(&mut q, f);
        q.push(" GROUP BY bucket ORDER BY bucket");
        let (sql, args) = numbered_query(&mut q);
        let rows = sqlx::query_with(&sql, args).fetch_all(&self.pool).await?;
        let mut buckets = (0..((f.end_ms - f.start_ms + bucket_ms - 1) / bucket_ms))
            .map(|i| PerformanceBucket {
                start_ms: f.start_ms + i * bucket_ms,
                counts: PerformanceCounts::default(),
                duration: PerformancePercentiles::default(),
                first_content: PerformancePercentiles::default(),
            })
            .collect::<Vec<_>>();
        let mut counts = PerformanceCounts::default();
        for row in rows {
            let c = read_counts(&row)?;
            counts.completed_count += c.completed_count;
            counts.success_count += c.success_count;
            counts.error_count += c.error_count;
            counts.aborted_count += c.aborted_count;
            counts.unknown_count += c.unknown_count;
            if let Some(bucket) = buckets.get_mut(row.try_get::<i64, _>("bucket")? as usize) {
                bucket.counts = c;
            }
        }
        let duration = self
            .percentiles(f, "duration_ms", None)
            .await?
            .pop()
            .map(|(_, p)| p)
            .unwrap_or_default();
        let first_byte = self
            .percentiles(f, "first_byte_ms", None)
            .await?
            .pop()
            .map(|(_, p)| p)
            .unwrap_or_default();
        let first_content = self
            .percentiles(f, "first_content_ms", None)
            .await?
            .pop()
            .map(|(_, p)| p)
            .unwrap_or_default();
        let first_text = self
            .percentiles(f, "first_text_ms", None)
            .await?
            .pop()
            .map(|(_, p)| p)
            .unwrap_or_default();
        let output_speed = self
            .percentiles(f, "output_speed", None)
            .await?
            .pop()
            .map(|(_, p)| p)
            .unwrap_or_default();
        for (id, p) in self
            .percentiles(f, "duration_ms", Some(&bucket_expr))
            .await?
        {
            if let Some(b) = buckets.get_mut(id as usize) {
                b.duration = p;
            }
        }
        for (id, p) in self
            .percentiles(f, "first_content_ms", Some(&bucket_expr))
            .await?
        {
            if let Some(b) = buckets.get_mut(id as usize) {
                b.first_content = p;
            }
        }
        let mut q = QueryBuilder::<Any>::new("SELECT ");
        for (i, (lower, upper, _)) in HISTOGRAM.iter().enumerate() {
            for (prefix, col) in [("d", "duration_ms"), ("c", "first_content_ms")] {
                if i > 0 || prefix == "c" {
                    q.push(", ");
                }
                q.push(format!("CAST(COALESCE(SUM(CASE WHEN outcome='success' AND {col}>={lower} AND {col}<{upper} THEN 1 ELSE 0 END),0) AS BIGINT) AS {prefix}{i}"));
            }
        }
        q.push(" FROM request_performance_events WHERE ");
        where_filter(&mut q, f);
        let (sql, args) = numbered_query(&mut q);
        let row = sqlx::query_with(&sql, args).fetch_one(&self.pool).await?;
        let histogram = HISTOGRAM
            .iter()
            .enumerate()
            .map(|(i, (_, _, label))| {
                Ok(PerformanceHistogram {
                    label: (*label).into(),
                    duration_count: row.try_get(format!("d{i}").as_str())?,
                    first_content_count: row.try_get(format!("c{i}").as_str())?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?;
        Ok(PerformanceReport {
            start_ms: f.start_ms,
            end_ms: f.end_ms,
            bucket_ms,
            counts,
            duration,
            first_byte,
            first_content,
            first_text,
            output_speed,
            buckets,
            histogram,
        })
    }

    async fn percentiles(
        &self,
        f: &PerformanceFilter,
        column: &str,
        bucket: Option<&str>,
    ) -> Result<Vec<(i64, PerformancePercentiles)>, AppError> {
        let bucket = bucket.unwrap_or("CAST(0 AS BIGINT)");
        let mut q = QueryBuilder::<Any>::new(format!(
            "WITH samples AS (SELECT {bucket} AS bucket, CAST({column} AS DOUBLE PRECISION) AS value FROM request_performance_events WHERE "
        ));
        where_filter(&mut q, f);
        q.push(format!(" AND outcome='success' AND {column} IS NOT NULL), ranked AS (SELECT bucket,value,ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY value) AS rn, COUNT(*) OVER (PARTITION BY bucket) AS n FROM samples) SELECT bucket, CAST(MAX(n) AS BIGINT) AS n, MAX(CASE WHEN rn=CAST((n*50+99)/100 AS BIGINT) THEN value END) AS p50, MAX(CASE WHEN rn=CAST((n*95+99)/100 AS BIGINT) THEN value END) AS p95, MAX(CASE WHEN rn=CAST((n*99+99)/100 AS BIGINT) THEN value END) AS p99, MAX(value) AS maximum FROM ranked GROUP BY bucket ORDER BY bucket"));
        let (sql, args) = numbered_query(&mut q);
        sqlx::query_with(&sql, args)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                Ok((
                    row.try_get("bucket")?,
                    PerformancePercentiles {
                        sample_count: row.try_get("n")?,
                        p50: row.try_get("p50")?,
                        p95: row.try_get("p95")?,
                        p99: row.try_get("p99")?,
                        max: row.try_get("maximum")?,
                    },
                ))
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(AppError::from)
    }
}

pub fn sg_day(ms: i64) -> i32 {
    ((ms + 28_800_000).div_euclid(86_400_000)) as i32
}

fn decode_event(json: &str) -> Result<PerformanceEvent, AppError> {
    serde_json::from_str(json)
        .map_err(|e| AppError::Internal(format!("read performance event: {e}")))
}

// Any's QueryBuilder emits `?`, which PostgreSQL does not accept. All `?` here
// come from push_bind; user strings are exclusively in the separate arguments.
fn numbered_query<'a>(q: &'a mut QueryBuilder<'a, Any>) -> (String, AnyArguments<'a>) {
    let mut index = 0;
    let sql = q
        .sql()
        .chars()
        .map(|c| {
            if c == '?' {
                index += 1;
                format!("${index}")
            } else {
                c.to_string()
            }
        })
        .collect::<String>();
    let args = q.build().take_arguments().unwrap_or_default();
    (sql, args)
}

fn where_filter(q: &mut QueryBuilder<'_, Any>, f: &PerformanceFilter) {
    q.push("sg_day >= ")
        .push_bind(sg_day(f.start_ms))
        .push(" AND sg_day <= ")
        .push_bind(sg_day(f.end_ms - 1));
    q.push(" AND started_at_ms >= ")
        .push_bind(f.start_ms)
        .push(" AND started_at_ms < ")
        .push_bind(f.end_ms);
    if let Some(id) = f.account_id {
        q.push(" AND account_id = ").push_bind(id);
    }
    if let Some(id) = f.api_token_id {
        q.push(" AND api_token_id = ").push_bind(id);
    }
    if let Some(v) = &f.model {
        q.push(" AND model = ").push_bind(v.clone());
    }
    if let Some(v) = &f.instance_id {
        q.push(" AND instance_id = ").push_bind(v.clone());
    }
    if let Some(v) = f.outcome {
        q.push(" AND outcome = ").push_bind(v.as_str());
    }
    if let Some(v) = f.is_stream {
        q.push(" AND is_stream = ").push_bind(i32::from(v));
    }
    if let Some(v) = f.min_duration_ms {
        q.push(" AND duration_ms >= ").push_bind(v);
    }
}

fn counts_sql() -> &'static str {
    "CAST(COUNT(*) AS BIGINT) AS completed_count, CAST(SUM(CASE WHEN outcome='success' THEN 1 ELSE 0 END) AS BIGINT) AS success_count, CAST(SUM(CASE WHEN outcome NOT IN ('success','aborted','unknown') THEN 1 ELSE 0 END) AS BIGINT) AS error_count, CAST(SUM(CASE WHEN outcome='aborted' THEN 1 ELSE 0 END) AS BIGINT) AS aborted_count, CAST(SUM(CASE WHEN outcome='unknown' THEN 1 ELSE 0 END) AS BIGINT) AS unknown_count"
}
fn read_counts(row: &AnyRow) -> Result<PerformanceCounts, sqlx::Error> {
    Ok(PerformanceCounts {
        completed_count: row.try_get("completed_count")?,
        success_count: row.try_get("success_count")?,
        error_count: row.try_get("error_count")?,
        aborted_count: row.try_get("aborted_count")?,
        unknown_count: row.try_get("unknown_count")?,
    })
}
const HISTOGRAM: [(i64, i64, &str); 8] = [
    (0, 1000, "<1秒"),
    (1000, 5000, "1–5秒"),
    (5000, 15000, "5–15秒"),
    (15000, 60000, "15–60秒"),
    (60000, 300000, "1–5分钟"),
    (300000, 900000, "5–15分钟"),
    (900000, 1800000, "15–30分钟"),
    (1800000, i64::MAX, "≥30分钟"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::performance::PerformanceOutcome as Outcome;
    use chrono::Utc;

    #[tokio::test]
    async fn migration_persistence_percentiles_pagination_and_retention() {
        sqlx::any::install_default_drivers();
        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        exercise_store(pool, "sqlite").await;
    }

    #[tokio::test]
    #[ignore = "requires an isolated PERFORMANCE_TEST_POSTGRES_DSN database"]
    async fn postgres_performance_queries_match_sqlite() {
        sqlx::any::install_default_drivers();
        let dsn = std::env::var("PERFORMANCE_TEST_POSTGRES_DSN")
            .expect("isolated test database required");
        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(1)
            .connect(&dsn)
            .await
            .unwrap();
        // A private schema makes rerunning this integration check safe within the test database.
        let schema = format!("performance_test_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(&format!("SET search_path TO {schema}"))
            .execute(&pool)
            .await
            .unwrap();
        exercise_store(pool.clone(), "postgres").await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&pool)
            .await
            .unwrap();
    }

    async fn exercise_store(pool: AnyPool, driver: &str) {
        // The existing database is stamped v3; migration must add performance without erasing it.
        sqlx::query("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO schema_migrations VALUES (3)")
            .execute(&pool)
            .await
            .unwrap();
        crate::store::db::migrate(&pool, driver).await.unwrap();
        crate::store::db::migrate(&pool, driver).await.unwrap();
        let store = PerformanceStore::new(pool.clone());
        let (svc, _rx) = crate::service::performance::test_service();
        let guard = svc.begin().unwrap();
        let base = guard.handle.snapshot();
        let mut events = Vec::new();
        for i in 1..=100 {
            let mut e = base.clone();
            e.request_id = format!("request-{i:03}");
            e.completed_at_utc = Some(Utc::now());
            e.outcome = Some(Outcome::Success);
            e.duration_ms = Some(i);
            e.first_content_ms = Some(i);
            e.first_text_ms = (i % 2 == 0).then_some(i);
            e.output_tokens_per_second = Some(i as f64);
            e.request_model = Some("test-model".into());
            events.push(e);
        }
        let mut failed = base.clone();
        failed.request_id = "failed".into();
        failed.outcome = Some(Outcome::SendTimeout);
        failed.duration_ms = Some(2_000_000);
        events.push(failed);
        assert_eq!(store.insert_batch(&events).await.unwrap(), 101);
        assert_eq!(store.insert_batch(&events).await.unwrap(), 0);
        let mut f = PerformanceFilter {
            start_ms: base.started_at_utc.timestamp_millis() - 1000,
            end_ms: Utc::now().timestamp_millis() + 1000,
            account_id: None,
            api_token_id: None,
            model: None,
            instance_id: None,
            outcome: None,
            is_stream: None,
            min_duration_ms: None,
            duration_desc: true,
            page: 1,
            page_size: 20,
        };
        let report = store.report(&f).await.unwrap();
        assert_eq!(report.counts.completed_count, 101);
        assert_eq!(report.counts.error_count, 1);
        assert_eq!(report.duration.sample_count, 100);
        assert_eq!(report.duration.p50, Some(50.0));
        assert_eq!(report.duration.p95, Some(95.0));
        assert_eq!(report.duration.p99, Some(99.0));
        assert_eq!(report.duration.max, Some(100.0));
        assert_eq!(report.first_text.sample_count, 50);
        assert_eq!(report.first_text.p95, Some(96.0));
        assert_eq!(report.histogram[0].duration_count, 100);
        assert_eq!(report.buckets[0].duration.p95, Some(95.0));
        let list = store.list(&f).await.unwrap();
        assert_eq!(list.total, 101);
        assert_eq!(list.items[0].request_id, "failed");
        assert_eq!(
            store
                .get("failed", sg_day(f.start_ms))
                .await
                .unwrap()
                .outcome,
            Some(Outcome::SendTimeout)
        );
        let dims = store.dimensions(sg_day(f.start_ms)).await.unwrap();
        assert_eq!(dims.models, vec!["test-model"]);
        f.model = Some("test-model".into());
        f.page = 2;
        assert_eq!(store.list(&f).await.unwrap().items[0].duration_ms, Some(80));
        f.model = Some("' OR 1=1 --".into());
        assert_eq!(store.list(&f).await.unwrap().total, 0);
        assert_eq!(store.report(&f).await.unwrap().duration.sample_count, 0);
        assert_eq!(
            store
                .delete_before(sg_day(Utc::now().timestamp_millis()) + 1)
                .await
                .unwrap(),
            101
        );
    }
}
