use super::AlertRecord;
use crate::edge::{PersistedEdgeState, severity_from_name, severity_name};
use crate::model::{AlertStatus, AlertTransition};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;

pub(super) async fn open_pool(path: &Path) -> Result<SqlitePool, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|error| format!("failed to open {}: {error}", path.display()))
}

pub(super) async fn initialize_schema(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS edge_state (
            key TEXT PRIMARY KEY,
            endpoint TEXT NOT NULL,
            node TEXT NOT NULL,
            source TEXT NOT NULL,
            service TEXT NOT NULL,
            metric_points INTEGER NOT NULL,
            spans INTEGER NOT NULL,
            log_records INTEGER NOT NULL,
            last_signal TEXT NOT NULL,
            last_metric TEXT NOT NULL,
            last_value TEXT NOT NULL,
            last_seen_unix_ms INTEGER NOT NULL,
            severity TEXT
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_edge_state_last_seen
        ON edge_state(last_seen_unix_ms DESC)
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS alert_state (
            key TEXT PRIMARY KEY,
            rule_id TEXT NOT NULL,
            severity TEXT NOT NULL,
            status TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            node TEXT NOT NULL,
            source TEXT NOT NULL,
            summary TEXT NOT NULL,
            detail TEXT NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS alert_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at_unix_ms INTEGER NOT NULL,
            transition TEXT NOT NULL,
            key TEXT NOT NULL,
            rule_id TEXT NOT NULL,
            severity TEXT NOT NULL,
            status TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            node TEXT NOT NULL,
            source TEXT NOT NULL,
            summary TEXT NOT NULL,
            detail TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub(super) async fn load_edge_states(pool: &SqlitePool) -> sqlx::Result<Vec<PersistedEdgeState>> {
    let rows = sqlx::query(
        r#"
        SELECT
            key,
            endpoint,
            node,
            source,
            service,
            metric_points,
            spans,
            log_records,
            last_signal,
            last_metric,
            last_value,
            last_seen_unix_ms,
            severity
        FROM edge_state
        ORDER BY last_seen_unix_ms DESC
        LIMIT 512
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut states = Vec::with_capacity(rows.len());
    for row in rows {
        let severity: Option<String> = row.try_get("severity")?;
        states.push(PersistedEdgeState {
            key: row.try_get("key")?,
            endpoint: row.try_get("endpoint")?,
            node: row.try_get("node")?,
            source: row.try_get("source")?,
            service: row.try_get("service")?,
            metric_points: row.try_get("metric_points")?,
            spans: row.try_get("spans")?,
            log_records: row.try_get("log_records")?,
            last_signal: row.try_get("last_signal")?,
            last_metric: row.try_get("last_metric")?,
            last_value: row.try_get("last_value")?,
            last_seen_unix_ms: row.try_get("last_seen_unix_ms")?,
            severity: severity.as_deref().and_then(severity_from_name),
        });
    }

    Ok(states)
}

pub(super) async fn upsert_edge_state(
    pool: &SqlitePool,
    edge: &PersistedEdgeState,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO edge_state (
            key,
            endpoint,
            node,
            source,
            service,
            metric_points,
            spans,
            log_records,
            last_signal,
            last_metric,
            last_value,
            last_seen_unix_ms,
            severity
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(key) DO UPDATE SET
            endpoint = excluded.endpoint,
            node = excluded.node,
            source = excluded.source,
            service = excluded.service,
            metric_points = excluded.metric_points,
            spans = excluded.spans,
            log_records = excluded.log_records,
            last_signal = excluded.last_signal,
            last_metric = excluded.last_metric,
            last_value = excluded.last_value,
            last_seen_unix_ms = excluded.last_seen_unix_ms,
            severity = excluded.severity
        "#,
    )
    .bind(&edge.key)
    .bind(&edge.endpoint)
    .bind(&edge.node)
    .bind(&edge.source)
    .bind(&edge.service)
    .bind(edge.metric_points)
    .bind(edge.spans)
    .bind(edge.log_records)
    .bind(&edge.last_signal)
    .bind(&edge.last_metric)
    .bind(&edge.last_value)
    .bind(edge.last_seen_unix_ms)
    .bind(edge.severity.map(severity_name))
    .execute(pool)
    .await?;

    Ok(())
}

pub(super) async fn delete_edge_state(pool: &SqlitePool, key: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM edge_state WHERE key = ?1")
        .bind(key)
        .execute(pool)
        .await?;

    Ok(())
}

pub(super) async fn record_alert(pool: &SqlitePool, alert: &AlertRecord) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO alert_events (
            at_unix_ms,
            transition,
            key,
            rule_id,
            severity,
            status,
            endpoint,
            node,
            source,
            summary,
            detail
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(alert.at_unix_ms)
    .bind(transition_name(alert.transition))
    .bind(&alert.key)
    .bind(&alert.rule_id)
    .bind(severity_name(alert.severity))
    .bind(status_name(alert.status))
    .bind(&alert.endpoint)
    .bind(&alert.node)
    .bind(&alert.source)
    .bind(&alert.summary)
    .bind(&alert.detail)
    .execute(pool)
    .await?;

    match alert.status {
        AlertStatus::Firing => upsert_alert_state(pool, alert).await?,
        AlertStatus::Resolved => {
            sqlx::query("DELETE FROM alert_state WHERE key = ?1")
                .bind(&alert.key)
                .execute(pool)
                .await?;
        }
    }

    Ok(())
}

async fn upsert_alert_state(pool: &SqlitePool, alert: &AlertRecord) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO alert_state (
            key,
            rule_id,
            severity,
            status,
            endpoint,
            node,
            source,
            summary,
            detail,
            updated_at_unix_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(key) DO UPDATE SET
            rule_id = excluded.rule_id,
            severity = excluded.severity,
            status = excluded.status,
            endpoint = excluded.endpoint,
            node = excluded.node,
            source = excluded.source,
            summary = excluded.summary,
            detail = excluded.detail,
            updated_at_unix_ms = excluded.updated_at_unix_ms
        "#,
    )
    .bind(&alert.key)
    .bind(&alert.rule_id)
    .bind(severity_name(alert.severity))
    .bind(status_name(alert.status))
    .bind(&alert.endpoint)
    .bind(&alert.node)
    .bind(&alert.source)
    .bind(&alert.summary)
    .bind(&alert.detail)
    .bind(alert.at_unix_ms)
    .execute(pool)
    .await?;

    Ok(())
}

fn transition_name(transition: AlertTransition) -> &'static str {
    match transition {
        AlertTransition::Fired => "fired",
        AlertTransition::Resolved => "resolved",
    }
}

fn status_name(status: AlertStatus) -> &'static str {
    match status {
        AlertStatus::Firing => "firing",
        AlertStatus::Resolved => "resolved",
    }
}
