use crate::ports::{UsageBucketRow, UsageReporter, UsageWindow};
use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::access_control::ScopedEntity;
use moatless_types::store::StoreError;

/// Scoped T3 usage buckets read from persisted Task turn metrics.
pub struct PgUsageDao {
    pool: sqlx::PgPool,
}

impl PgUsageDao {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UsageReporter for PgUsageDao {
    async fn buckets(
        &self,
        viewer: &ScopeContext,
        window: &UsageWindow,
    ) -> Result<Vec<UsageBucketRow>, StoreError> {
        // `modelUsage` is Claude's per-model breakdown and is the only place a
        // model name survives on a Claude turn. Codex reports one model per
        // run and no breakdown, so its row is built from `metrics -> usage`
        // with `metrics ->> 'model'` as the name. The two arms are unioned
        // rather than coalesced: a Claude turn using two models has to fan out
        // into two rows, and a Codex turn must not.
        let access = moatless_core::db::access_control::access_filter(ScopedEntity::Task, "k.", 4);
        // Interpolated into `cells`, which selects `FROM turns`: name the CTE's
        // columns unqualified. A `t.` prefix resolves in `turns` but not here,
        // and Postgres rejects the whole statement with "missing FROM-clause
        // entry for table t".
        let bucket_expr = if window.hourly {
            "to_char(date_trunc('hour', started_at AT TIME ZONE $3), 'YYYY-MM-DD\"T\"HH24:00:00\"Z\"')"
        } else {
            "NULL"
        };
        let sql = format!(
            r#"
            WITH turns AS (
                SELECT t.task_id, t.started_at, t.metrics,
                       CASE WHEN k.agent_type LIKE 'claude%' THEN 'claude'
                            WHEN k.agent_type LIKE 'codex%'  THEN 'codex'
                            ELSE NULL END AS provider
                FROM task_turns t
                JOIN tasks k ON k.task_id = t.task_id
                WHERE (t.started_at AT TIME ZONE $3)::date BETWEEN $1::date AND $2::date
                  AND {access}
            ),
            cells AS (
                SELECT provider,
                       to_char(started_at AT TIME ZONE $3, 'YYYY-MM-DD') AS day,
                       {bucket_expr} AS hour_start,
                       m.key AS model,
                       task_id,
                       (m.value->>'inputTokens')::bigint             AS uncached_input,
                       (m.value->>'cacheReadInputTokens')::bigint     AS cached_input,
                       (m.value->>'cacheCreationInputTokens')::bigint AS cache_creation,
                       (m.value->>'outputTokens')::bigint             AS output,
                       0::bigint                                      AS reasoning,
                       COALESCE((m.value->>'costUSD')::float8, 0)     AS cost,
                       ((m.value->>'costUSD') IS NULL)::int           AS unpriced
                FROM turns, jsonb_each(metrics->'modelUsage') m
                WHERE provider IS NOT NULL AND jsonb_typeof(metrics->'modelUsage') = 'object'
                UNION ALL
                SELECT provider,
                       to_char(started_at AT TIME ZONE $3, 'YYYY-MM-DD') AS day,
                       {bucket_expr} AS hour_start,
                       COALESCE(metrics->>'model', 'unknown') AS model,
                       task_id,
                       (metrics->'usage'->>'input_tokens')::bigint             AS uncached_input,
                       (metrics->'usage'->>'cache_read_input_tokens')::bigint   AS cached_input,
                       (metrics->'usage'->>'cache_creation_input_tokens')::bigint AS cache_creation,
                       (metrics->'usage'->>'output_tokens')::bigint             AS output,
                       COALESCE((metrics->'usage'->>'reasoning_output_tokens')::bigint, 0) AS reasoning,
                       COALESCE((metrics->>'totalCostUsd')::float8, 0)          AS cost,
                       ((metrics->>'totalCostUsd') IS NULL)::int                AS unpriced
                FROM turns
                WHERE provider IS NOT NULL
                  AND jsonb_typeof(metrics->'modelUsage') IS DISTINCT FROM 'object'
                  AND jsonb_typeof(metrics->'usage') = 'object'
            )
            SELECT day, hour_start, provider, model,
                   COALESCE(SUM(uncached_input), 0)::bigint AS uncached_input,
                   COALESCE(SUM(cached_input), 0)::bigint   AS cached_input,
                   COALESCE(SUM(cache_creation), 0)::bigint AS cache_creation,
                   COALESCE(SUM(output), 0)::bigint         AS output,
                   COALESCE(SUM(reasoning), 0)::bigint      AS reasoning,
                   COALESCE(SUM(cost), 0)::float8           AS cost,
                   COUNT(*)::bigint                         AS records,
                   COALESCE(SUM(unpriced), 0)::bigint       AS unpriced_records,
                   COUNT(DISTINCT task_id)::bigint          AS sessions
            FROM cells
            GROUP BY day, hour_start, provider, model
            ORDER BY day, provider, model
            "#,
            access = access.sql(),
        );

        let rows = sqlx::query_as::<_, UsageRow>(sqlx::AssertSqlSafe(sql))
            .bind(&window.since_day)
            .bind(&window.until_day)
            .bind(&window.time_zone)
            .bind(viewer.is_admin)
            .bind(&viewer.user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|cause| StoreError::with_source("read t3 usage buckets", cause))?;

        Ok(rows.into_iter().map(UsageRow::into_bucket).collect())
    }
}

#[derive(sqlx::FromRow)]
struct UsageRow {
    day: String,
    hour_start: Option<String>,
    provider: String,
    model: String,
    uncached_input: i64,
    cached_input: i64,
    cache_creation: i64,
    output: i64,
    reasoning: i64,
    cost: f64,
    records: i64,
    unpriced_records: i64,
    sessions: i64,
}

impl UsageRow {
    fn into_bucket(self) -> UsageBucketRow {
        UsageBucketRow {
            day: self.day,
            hour_start: self.hour_start,
            provider: self.provider,
            model: self.model,
            uncached_input_tokens: self.uncached_input,
            cached_input_tokens: self.cached_input,
            cache_creation_tokens: self.cache_creation,
            output_tokens: self.output,
            reasoning_tokens: self.reasoning,
            cost_usd: self.cost,
            records: self.records,
            unpriced_records: self.unpriced_records,
            sessions: self.sessions,
        }
    }
}
