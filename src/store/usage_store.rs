use sqlx::any::AnyRow;
use sqlx::{Any, AnyPool, QueryBuilder, Row};

use crate::error::AppError;
use crate::model::usage::{
    DailyUsageRow, UnpricedUsageEvent, UsageDimensionOption, UsageEvent, UsageFilters,
    UsageGroupBy, UsageMetrics, UsagePricingUpdate, UsageTokens,
};

pub struct UsageStore {
    pool: AnyPool,
    driver: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UsageDimensions {
    pub accounts: Vec<UsageDimensionOption>,
    pub api_tokens: Vec<UsageDimensionOption>,
    pub models: Vec<String>,
}

impl UsageStore {
    pub fn new(pool: AnyPool, driver: String) -> Self {
        Self { pool, driver }
    }

    pub async fn insert_batch(&self, events: &[UsageEvent]) -> Result<(u64, u64), AppError> {
        if events.is_empty() {
            return Ok((0, 0));
        }
        let mut tx = self.pool.begin().await?;
        let mut inserted = 0_u64;
        let statement = self.insert_statement();
        for event in events {
            let occurred_at = event.occurred_at_utc.to_rfc3339();
            let result = sqlx::query(&statement)
                .bind(&event.dedup_key)
                .bind(&event.upstream_message_id)
                .bind(&event.upstream_request_id)
                .bind(&occurred_at)
                .bind(event.sg_day)
                .bind(event.account_id)
                .bind(event.api_token_id)
                .bind(&event.model)
                .bind(event.tokens.input)
                .bind(event.tokens.output)
                .bind(event.tokens.cache_creation_5m)
                .bind(event.tokens.cache_creation_1h)
                .bind(event.tokens.cache_read)
                .bind(event.costs.input_nano_usd)
                .bind(event.costs.output_nano_usd)
                .bind(event.costs.cache_creation_5m_nano_usd)
                .bind(event.costs.cache_creation_1h_nano_usd)
                .bind(event.costs.cache_read_nano_usd)
                .bind(event.costs.known_nano_usd)
                .bind(i32::from(event.costs.complete))
                .bind(&event.pricing_version)
                .bind(&event.pricing_model_key)
                .bind(i32::from(event.http_status))
                .bind(i32::from(event.is_stream))
                .execute(&mut *tx)
                .await?;
            inserted += result.rows_affected();
        }
        tx.commit().await?;
        Ok((inserted, events.len() as u64 - inserted))
    }

    pub async fn aggregate_daily(
        &self,
        filters: &UsageFilters,
    ) -> Result<Vec<DailyUsageRow>, AppError> {
        let (group_id, group_text, group_column) = match filters.group_by {
            UsageGroupBy::Account => ("account_id", "''", "account_id"),
            UsageGroupBy::ApiToken => ("api_token_id", "''", "api_token_id"),
            UsageGroupBy::Model => {
                let group_id = if self.driver == "postgres" {
                    "CAST(0 AS BIGINT)"
                } else {
                    "0"
                };
                (group_id, "model", "model")
            }
        };
        let mut qb = QueryBuilder::<Any>::new(format!(
            r#"SELECT sg_day, {group_id} AS group_id, {group_text} AS group_text,
                CAST(COUNT(*) AS TEXT) AS request_count,
                CAST(COALESCE(SUM(input_tokens), 0) AS TEXT) AS input_tokens,
                CAST(COALESCE(SUM(output_tokens), 0) AS TEXT) AS output_tokens,
                CAST(COALESCE(SUM(cache_creation_5m_tokens), 0) AS TEXT) AS cache_creation_5m_tokens,
                CAST(COALESCE(SUM(cache_creation_1h_tokens), 0) AS TEXT) AS cache_creation_1h_tokens,
                CAST(COALESCE(SUM(cache_read_tokens), 0) AS TEXT) AS cache_read_tokens,
                CAST(COALESCE(SUM(known_cost_nano_usd), 0) AS TEXT) AS known_cost_nano_usd,
                CAST(COALESCE(SUM(CASE WHEN cost_complete = 0 THEN 1 ELSE 0 END), 0) AS TEXT) AS unpriced_request_count,
                CAST(COALESCE(SUM(
                    CASE WHEN input_cost_nano_usd IS NULL THEN input_tokens ELSE 0 END +
                    CASE WHEN output_cost_nano_usd IS NULL THEN output_tokens ELSE 0 END +
                    CASE WHEN cache_creation_5m_cost_nano_usd IS NULL THEN cache_creation_5m_tokens ELSE 0 END +
                    CASE WHEN cache_creation_1h_cost_nano_usd IS NULL THEN cache_creation_1h_tokens ELSE 0 END +
                    CASE WHEN cache_read_cost_nano_usd IS NULL THEN cache_read_tokens ELSE 0 END
                ), 0) AS TEXT) AS unpriced_tokens
              FROM usage_events WHERE sg_day >= "#,
        ));
        qb.push_bind(filters.start_sg_day)
            .push(" AND sg_day <= ")
            .push_bind(filters.end_sg_day);
        if let Some(id) = filters.account_id {
            qb.push(" AND account_id = ").push_bind(id);
        }
        if let Some(id) = filters.api_token_id {
            qb.push(" AND api_token_id = ").push_bind(id);
        }
        if let Some(model) = &filters.model {
            qb.push(" AND model = ").push_bind(model);
        }
        qb.push(format!(
            " GROUP BY sg_day, {group_column} ORDER BY sg_day, {group_column}"
        ));
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter()
            .map(row_to_daily)
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)
    }

    pub async fn dimensions(&self, min_sg_day: i32) -> Result<UsageDimensions, AppError> {
        let accounts = sqlx::query(
            r#"SELECT ids.id AS id,
                COALESCE(NULLIF(a.name, ''), '账号 #' || CAST(ids.id AS TEXT)) AS label
              FROM (
                SELECT DISTINCT account_id AS id FROM usage_events WHERE sg_day >= $1
                UNION SELECT id FROM accounts
              ) ids LEFT JOIN accounts a ON a.id = ids.id
              ORDER BY label"#,
        )
        .bind(min_sg_day)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(row_to_dimension)
        .collect::<Result<Vec<_>, _>>()?;
        let api_tokens = sqlx::query(
            r#"SELECT ids.id AS id,
                COALESCE(NULLIF(t.name, ''), 'Token #' || CAST(ids.id AS TEXT)) AS label
              FROM (
                SELECT DISTINCT api_token_id AS id FROM usage_events WHERE sg_day >= $1
                UNION SELECT id FROM api_tokens
              ) ids LEFT JOIN api_tokens t ON t.id = ids.id
              ORDER BY label"#,
        )
        .bind(min_sg_day)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(row_to_dimension)
        .collect::<Result<Vec<_>, _>>()?;
        let models = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT model FROM usage_events WHERE sg_day >= $1 ORDER BY model",
        )
        .bind(min_sg_day)
        .fetch_all(&self.pool)
        .await?;
        Ok(UsageDimensions {
            accounts,
            api_tokens,
            models,
        })
    }

    pub async fn delete_before(&self, min_sg_day: i32, batch_size: i64) -> Result<u64, AppError> {
        let result = sqlx::query(
            "DELETE FROM usage_events WHERE id IN (SELECT id FROM usage_events WHERE sg_day < $1 ORDER BY id LIMIT $2)",
        )
        .bind(min_sg_day)
        .bind(batch_size)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn load_pricing_snapshot(&self) -> Result<Option<String>, AppError> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT snapshot_json FROM usage_pricing_cache WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn save_pricing_snapshot(
        &self,
        snapshot_json: &str,
        pricing_version: &str,
        fetched_at_utc: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AppError> {
        let fetched_at = fetched_at_utc.to_rfc3339();
        let fetched_at_bind = if self.driver == "postgres" {
            "$4::TEXT::TIMESTAMPTZ"
        } else {
            "$4"
        };
        let statement = format!(
            "INSERT INTO usage_pricing_cache (id, snapshot_json, pricing_version, fetched_at_utc) \
             VALUES ($1, $2, $3, {fetched_at_bind}) \
             ON CONFLICT (id) DO UPDATE SET snapshot_json = EXCLUDED.snapshot_json, \
             pricing_version = EXCLUDED.pricing_version, fetched_at_utc = EXCLUDED.fetched_at_utc"
        );
        sqlx::query(&statement)
            .bind(1_i32)
            .bind(snapshot_json)
            .bind(pricing_version)
            .bind(fetched_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn unpriced_events_after(
        &self,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<UnpricedUsageEvent>, AppError> {
        let rows = sqlx::query(
            r#"SELECT id, model, input_tokens, output_tokens,
                      cache_creation_5m_tokens, cache_creation_1h_tokens, cache_read_tokens
               FROM usage_events
               WHERE cost_complete = 0 AND id > $1
               ORDER BY id LIMIT $2"#,
        )
        .bind(after_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(UnpricedUsageEvent {
                    id: row.try_get("id")?,
                    model: row.try_get("model")?,
                    tokens: UsageTokens {
                        input: row.try_get("input_tokens")?,
                        output: row.try_get("output_tokens")?,
                        cache_creation_5m: row.try_get("cache_creation_5m_tokens")?,
                        cache_creation_1h: row.try_get("cache_creation_1h_tokens")?,
                        cache_read: row.try_get("cache_read_tokens")?,
                    },
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(AppError::from)
    }

    pub async fn apply_pricing_updates(
        &self,
        updates: &[UsagePricingUpdate],
    ) -> Result<u64, AppError> {
        if updates.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await?;
        let mut updated = 0_u64;
        let nullable = if self.driver == "postgres" {
            [
                "$2::BIGINT",
                "$3::BIGINT",
                "$4::BIGINT",
                "$5::BIGINT",
                "$6::BIGINT",
            ]
        } else {
            ["$2", "$3", "$4", "$5", "$6"]
        };
        let statement = format!(
            r#"UPDATE usage_events SET
                   input_cost_nano_usd = {}, output_cost_nano_usd = {},
                   cache_creation_5m_cost_nano_usd = {},
                   cache_creation_1h_cost_nano_usd = {}, cache_read_cost_nano_usd = {},
                   known_cost_nano_usd = $7, cost_complete = $8,
                   pricing_version = $9, pricing_model_key = $10
               WHERE id = $1 AND cost_complete = 0"#,
            nullable[0], nullable[1], nullable[2], nullable[3], nullable[4]
        );
        for update in updates {
            let result = sqlx::query(&statement)
                .bind(update.id)
                .bind(update.costs.input_nano_usd)
                .bind(update.costs.output_nano_usd)
                .bind(update.costs.cache_creation_5m_nano_usd)
                .bind(update.costs.cache_creation_1h_nano_usd)
                .bind(update.costs.cache_read_nano_usd)
                .bind(update.costs.known_nano_usd)
                .bind(i32::from(update.costs.complete))
                .bind(&update.pricing_version)
                .bind(&update.pricing_model_key)
                .execute(&mut *tx)
                .await?;
            updated += result.rows_affected();
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub fn driver(&self) -> &str {
        &self.driver
    }

    fn insert_statement(&self) -> String {
        let bindings = if self.driver == "postgres" {
            [
                "$2::TEXT",
                "$3::TEXT",
                "$4::TEXT::TIMESTAMPTZ",
                "$14::BIGINT",
                "$15::BIGINT",
                "$16::BIGINT",
                "$17::BIGINT",
                "$18::BIGINT",
                "$22::TEXT",
            ]
        } else {
            ["$2", "$3", "$4", "$14", "$15", "$16", "$17", "$18", "$22"]
        };
        let [
            message_id,
            request_id,
            occurred_at,
            input_cost,
            output_cost,
            cache_5m_cost,
            cache_1h_cost,
            cache_read_cost,
            pricing_model,
        ] = bindings;
        format!(
            r#"INSERT INTO usage_events (
                dedup_key, upstream_message_id, upstream_request_id, occurred_at_utc, sg_day,
                account_id, api_token_id, model, input_tokens, output_tokens,
                cache_creation_5m_tokens, cache_creation_1h_tokens, cache_read_tokens,
                input_cost_nano_usd, output_cost_nano_usd,
                cache_creation_5m_cost_nano_usd, cache_creation_1h_cost_nano_usd,
                cache_read_cost_nano_usd, known_cost_nano_usd, cost_complete,
                pricing_version, pricing_model_key, http_status, is_stream
            ) VALUES (
                $1,{message_id},{request_id},{occurred_at},$5,$6,$7,$8,$9,$10,$11,$12,$13,{input_cost},{output_cost},{cache_5m_cost},{cache_1h_cost},{cache_read_cost},$19,$20,$21,{pricing_model},$23,$24
            ) ON CONFLICT (dedup_key) DO NOTHING"#
        )
    }
}

fn row_to_daily(row: &AnyRow) -> Result<DailyUsageRow, sqlx::Error> {
    let unpriced_request_count = row_i64_text(row, "unpriced_request_count")?;
    Ok(DailyUsageRow {
        sg_day: row.try_get("sg_day")?,
        group_id: row.try_get("group_id")?,
        group_text: row.try_get("group_text")?,
        metrics: UsageMetrics {
            request_count: row_i64_text(row, "request_count")?,
            tokens: UsageTokens {
                input: row_i64_text(row, "input_tokens")?,
                output: row_i64_text(row, "output_tokens")?,
                cache_creation_5m: row_i64_text(row, "cache_creation_5m_tokens")?,
                cache_creation_1h: row_i64_text(row, "cache_creation_1h_tokens")?,
                cache_read: row_i64_text(row, "cache_read_tokens")?,
            },
            known_cost_nano_usd: row_i64_text(row, "known_cost_nano_usd")?,
            cost_complete: unpriced_request_count == 0,
            unpriced_request_count,
            unpriced_tokens: row_i64_text(row, "unpriced_tokens")?,
        },
    })
}

fn row_i64_text(row: &AnyRow, column: &str) -> Result<i64, sqlx::Error> {
    row.try_get::<String, _>(column)?
        .parse::<i64>()
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn row_to_dimension(row: &AnyRow) -> Result<UsageDimensionOption, sqlx::Error> {
    Ok(UsageDimensionOption {
        id: row.try_get("id")?,
        label: row.try_get("label")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::usage::{UsageCosts, UsageTokens};
    use chrono::{TimeZone, Utc};

    async fn sqlite_store() -> (UsageStore, std::path::PathBuf) {
        sqlx::any::install_default_drivers();
        let path =
            std::env::temp_dir().join(format!("ccbridge_usage_{}.db", rand::random::<u64>()));
        let pool = crate::store::db::init_db("sqlite", path.to_str().unwrap())
            .await
            .unwrap();
        crate::store::db::migrate(&pool, "sqlite").await.unwrap();
        (UsageStore::new(pool, "sqlite".into()), path)
    }

    fn event(key: &str, day: i32, account_id: i64, token_id: i64, model: &str) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            upstream_message_id: Some(format!("msg_{key}")),
            upstream_request_id: None,
            occurred_at_utc: Utc
                .timestamp_opt(1_700_000_000 + i64::from(day), 0)
                .unwrap(),
            sg_day: day,
            account_id,
            api_token_id: token_id,
            model: model.into(),
            tokens: UsageTokens {
                input: 10,
                output: 2,
                cache_creation_5m: 3,
                cache_creation_1h: 4,
                cache_read: 5,
            },
            costs: UsageCosts {
                input_nano_usd: Some(10),
                output_nano_usd: Some(20),
                cache_creation_5m_nano_usd: Some(30),
                cache_creation_1h_nano_usd: Some(40),
                cache_read_nano_usd: Some(50),
                known_nano_usd: 150,
                complete: true,
                unpriced_tokens: 0,
            },
            pricing_version: "test-v1".into(),
            pricing_model_key: Some(model.into()),
            http_status: 200,
            is_stream: true,
        }
    }

    async fn remove_sqlite(store: UsageStore, path: &std::path::Path) {
        store.pool.close().await;
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn sqlite_contract_covers_migration_dedup_aggregate_dimensions_and_retention() {
        let (store, path) = sqlite_store().await;
        let version: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations WHERE version = 3")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(version, 1);

        sqlx::query(
            "INSERT INTO accounts (id, name, email, token, device_id) VALUES (1, '主账号', 'a@example.com', 'secret', 'device')",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO api_tokens (id, name, token) VALUES (2, '生产 Token', 'sk-secret')",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        let first = event("one", 100, 1, 2, "claude-sonnet-4-6");
        let mut second = event("two", 101, 9, 8, "unknown-model");
        second.costs.output_nano_usd = None;
        second.costs.known_nano_usd = 130;
        second.costs.complete = false;
        second.costs.unpriced_tokens = 2;
        assert_eq!(
            store.insert_batch(&[first.clone(), first]).await.unwrap(),
            (1, 1)
        );
        assert_eq!(store.insert_batch(&[second]).await.unwrap(), (1, 0));

        let rows = store
            .aggregate_daily(&UsageFilters {
                start_sg_day: 100,
                end_sg_day: 101,
                model: Some("claude-sonnet-4-6".into()),
                group_by: UsageGroupBy::Model,
                ..UsageFilters::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].group_text, "claude-sonnet-4-6");
        assert_eq!(rows[0].metrics.request_count, 1);
        assert_eq!(rows[0].metrics.tokens.total(), 24);
        assert_eq!(rows[0].metrics.known_cost_nano_usd, 150);

        let unpriced = store
            .aggregate_daily(&UsageFilters {
                start_sg_day: 100,
                end_sg_day: 101,
                model: Some("unknown-model".into()),
                group_by: UsageGroupBy::Model,
                ..UsageFilters::default()
            })
            .await
            .unwrap();
        assert_eq!(unpriced[0].metrics.known_cost_nano_usd, 130);
        assert_eq!(unpriced[0].metrics.unpriced_request_count, 1);
        assert_eq!(unpriced[0].metrics.unpriced_tokens, 2);
        assert!(!unpriced[0].metrics.cost_complete);

        store
            .save_pricing_snapshot(
                r#"{"version":"remote-v1","rules":{"unknown-model":{"model_key":"unknown-model","input":1000000000,"output":2000000000,"cache_creation_5m":3000000000,"cache_creation_1h":4000000000,"cache_read":5000000000,"long_context":null}}}"#,
                "remote-v1",
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            store
                .load_pricing_snapshot()
                .await
                .unwrap()
                .unwrap()
                .contains("remote-v1")
        );

        let unpriced_events = store.unpriced_events_after(0, 10).await.unwrap();
        assert_eq!(unpriced_events.len(), 1);
        let unpriced_id = unpriced_events[0].id;
        let costs = UsageCosts {
            input_nano_usd: Some(10),
            output_nano_usd: Some(20),
            cache_creation_5m_nano_usd: Some(30),
            cache_creation_1h_nano_usd: Some(40),
            cache_read_nano_usd: Some(50),
            known_nano_usd: i64::from(i32::MAX) + 10,
            complete: true,
            unpriced_tokens: 0,
        };
        let update = UsagePricingUpdate {
            id: unpriced_id,
            costs: costs.clone(),
            pricing_version: "remote-v1".into(),
            pricing_model_key: "unknown-model".into(),
        };
        assert_eq!(
            store
                .apply_pricing_updates(&[update.clone()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(store.apply_pricing_updates(&[update]).await.unwrap(), 0);
        let stored_cost: String = sqlx::query_scalar(
            "SELECT CAST(known_cost_nano_usd AS TEXT) FROM usage_events WHERE id = $1",
        )
        .bind(unpriced_id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(stored_cost.parse::<i64>().unwrap(), costs.known_nano_usd);

        let dimensions = store.dimensions(100).await.unwrap();
        assert!(
            dimensions
                .accounts
                .iter()
                .any(|item| item.id == 1 && item.label == "主账号")
        );
        assert!(
            dimensions
                .accounts
                .iter()
                .any(|item| item.id == 9 && item.label == "账号 #9")
        );
        assert!(
            dimensions
                .api_tokens
                .iter()
                .any(|item| item.id == 2 && item.label == "生产 Token")
        );
        assert!(
            dimensions
                .api_tokens
                .iter()
                .any(|item| item.id == 8 && item.label == "Token #8")
        );

        assert_eq!(store.delete_before(101, 1).await.unwrap(), 1);
        assert_eq!(store.delete_before(101, 1).await.unwrap(), 0);
        remove_sqlite(store, &path).await;
    }

    #[tokio::test]
    async fn postgres_insert_casts_nullable_text_and_timestamp() {
        let store = UsageStore::new(
            sqlx::AnyPool::connect_lazy("sqlite::memory:").unwrap(),
            "postgres".into(),
        );
        let sql = store.insert_statement();
        assert!(sql.contains("$2::TEXT"));
        assert!(sql.contains("$3::TEXT"));
        assert!(sql.contains("$4::TEXT::TIMESTAMPTZ"));
        for parameter in 14..=18 {
            assert!(sql.contains(&format!("${parameter}::BIGINT")));
        }
        assert!(sql.contains("$22::TEXT"));
    }
}
