use crate::config;
use crate::edge::{PersistedEdgeState, severity_from_name, severity_name};
use crate::model::{AlertChange, AlertSeverity, AlertStatus, AlertTransition};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORAGE_INIT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct StorageInit {
    pub(crate) storage: Option<StateStorage>,
    pub(crate) restored_edges: Vec<PersistedEdgeState>,
    pub(crate) message: Option<String>,
}

#[derive(Clone)]
pub(crate) struct StateStorage {
    tx: Sender<StorageCommand>,
}

impl StateStorage {
    pub(crate) fn start() -> StorageInit {
        let path = state_db_path();
        let label = path.display().to_string();
        let (tx, rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::channel();
        thread::spawn(move || run_storage(path, rx, init_tx));

        match init_rx.recv_timeout(STORAGE_INIT_TIMEOUT) {
            Ok(Ok(restored_edges)) => StorageInit {
                storage: Some(Self { tx }),
                restored_edges,
                message: Some(format!("state db ready at {label}")),
            },
            Ok(Err(error)) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                message: Some(format!("state db disabled: {error}")),
            },
            Err(error) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                message: Some(format!("state db startup timed out: {error}")),
            },
        }
    }

    pub(crate) fn persist_edge(&self, edge: &PersistedEdgeState) {
        let _ = self.tx.send(StorageCommand::UpsertEdge(edge.clone()));
    }

    pub(crate) fn persist_alert(&self, change: &AlertChange) {
        let _ = self
            .tx
            .send(StorageCommand::RecordAlert(AlertRecord::from(change)));
    }
}

enum StorageCommand {
    UpsertEdge(PersistedEdgeState),
    RecordAlert(AlertRecord),
}

struct AlertRecord {
    at_unix_ms: i64,
    transition: AlertTransition,
    key: String,
    rule_id: String,
    severity: AlertSeverity,
    status: AlertStatus,
    endpoint: String,
    node: String,
    source: String,
    summary: String,
    detail: String,
}

impl From<&AlertChange> for AlertRecord {
    fn from(change: &AlertChange) -> Self {
        Self {
            at_unix_ms: unix_millis(),
            transition: change.transition,
            key: change.snapshot.key.clone(),
            rule_id: change.snapshot.rule_id.clone(),
            severity: change.snapshot.severity,
            status: change.snapshot.status,
            endpoint: change.snapshot.endpoint.clone(),
            node: change.snapshot.node.clone(),
            source: change.snapshot.source.clone(),
            summary: change.snapshot.summary.clone(),
            detail: change.snapshot.detail.clone(),
        }
    }
}

fn run_storage(
    path: PathBuf,
    rx: Receiver<StorageCommand>,
    init_tx: Sender<Result<Vec<PersistedEdgeState>, String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = init_tx.send(Err(format!("failed to start sqlite runtime: {error}")));
            return;
        }
    };

    let pool = match runtime.block_on(open_pool(&path)) {
        Ok(pool) => pool,
        Err(error) => {
            let _ = init_tx.send(Err(error));
            return;
        }
    };

    if let Err(error) = runtime.block_on(initialize_schema(&pool)) {
        let _ = init_tx.send(Err(error.to_string()));
        return;
    }

    let restored_edges = match runtime.block_on(load_edge_states(&pool)) {
        Ok(edges) => edges,
        Err(error) => {
            let _ = init_tx.send(Err(error.to_string()));
            return;
        }
    };

    if init_tx.send(Ok(restored_edges)).is_err() {
        return;
    }

    while let Ok(command) = rx.recv() {
        let result = match command {
            StorageCommand::UpsertEdge(edge) => runtime.block_on(upsert_edge_state(&pool, &edge)),
            StorageCommand::RecordAlert(alert) => runtime.block_on(record_alert(&pool, &alert)),
        };
        if let Err(error) = result {
            eprintln!("skid-monitor-fe state db write failed: {error}");
        }
    }
}

async fn open_pool(path: &Path) -> Result<SqlitePool, String> {
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

async fn initialize_schema(pool: &SqlitePool) -> sqlx::Result<()> {
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

async fn load_edge_states(pool: &SqlitePool) -> sqlx::Result<Vec<PersistedEdgeState>> {
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

async fn upsert_edge_state(pool: &SqlitePool, edge: &PersistedEdgeState) -> sqlx::Result<()> {
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

async fn record_alert(pool: &SqlitePool, alert: &AlertRecord) -> sqlx::Result<()> {
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

fn state_db_path() -> PathBuf {
    if let Ok(path) = std::env::var(config::STATE_DB_PATH_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        let trimmed = state_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join("skid-monitor")
                .join(config::STATE_DB_DEFAULT_FILE);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join(".local")
                .join("state")
                .join("skid-monitor")
                .join(config::STATE_DB_DEFAULT_FILE);
        }
    }

    PathBuf::from(config::STATE_DB_DEFAULT_FILE)
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

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::edge_key;

    #[test]
    fn sqlite_edge_state_round_trips() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let path = temp_db_path("edge-round-trip");

        runtime.block_on(async {
            let pool = open_pool(&path).await.expect("open sqlite");
            initialize_schema(&pool).await.expect("schema");
            let edge = PersistedEdgeState {
                key: edge_key("127.0.0.1:9000", "edge-a"),
                endpoint: "127.0.0.1:9000".to_string(),
                node: "edge-a".to_string(),
                source: "edge_device".to_string(),
                service: "skid-edge-agent".to_string(),
                metric_points: 3,
                spans: 0,
                log_records: 0,
                last_signal: "metrics".to_string(),
                last_metric: "edge.temperature".to_string(),
                last_value: "31.5".to_string(),
                last_seen_unix_ms: unix_millis(),
                severity: Some(AlertSeverity::Critical),
            };

            upsert_edge_state(&pool, &edge).await.expect("upsert edge");
            let rows = load_edge_states(&pool).await.expect("load edges");

            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].key, edge.key);
            assert_eq!(rows[0].severity, Some(AlertSeverity::Critical));
        });

        cleanup_temp_db(&path);
    }

    fn temp_db_path(name: &str) -> PathBuf {
        let suffix = unix_millis();
        std::env::temp_dir().join(format!(
            "skid-monitor-fe-{name}-{}-{suffix}.sqlite3",
            std::process::id()
        ))
    }

    fn cleanup_temp_db(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(PathBuf::from(format!("{}{}", path.display(), suffix)));
        }
    }
}
