use super::db::{initialize_schema, load_edge_states, open_pool, upsert_edge_state};
use super::unix_millis;
use crate::edge::{PersistedEdgeState, edge_key};
use crate::model::AlertSeverity;
use std::path::{Path, PathBuf};

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
