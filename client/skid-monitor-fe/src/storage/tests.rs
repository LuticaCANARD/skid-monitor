use super::command::{StorageCommand, StorageControlCommand};
use super::db::{
    AVATAR_PROFILE_KEY, delete_edge_state, initialize_schema, load_avatar_profile,
    load_edge_states, open_pool, save_avatar_profile, upsert_edge_state,
};
use super::unix_millis;
use super::worker::run_storage;
use crate::edge::{PersistedEdgeState, edge_key};
use crate::model::{AlertSeverity, AvatarMotion, AvatarReactionProfile};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

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

#[test]
fn sqlite_edge_state_delete_removes_persisted_agent() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let path = temp_db_path("edge-delete");

    runtime.block_on(async {
        let pool = open_pool(&path).await.expect("open sqlite");
        initialize_schema(&pool).await.expect("schema");
        let edge = PersistedEdgeState {
            key: edge_key("127.0.0.1:9001", "edge-delete"),
            endpoint: "127.0.0.1:9001".to_string(),
            node: "edge-delete".to_string(),
            source: "manual".to_string(),
            service: "skid-monitor-agent".to_string(),
            metric_points: 0,
            spans: 0,
            log_records: 0,
            last_signal: "manual".to_string(),
            last_metric: "registered".to_string(),
            last_value: "pending".to_string(),
            last_seen_unix_ms: unix_millis(),
            severity: None,
        };

        upsert_edge_state(&pool, &edge).await.expect("upsert edge");
        delete_edge_state(&pool, &edge.key)
            .await
            .expect("delete edge");
        let rows = load_edge_states(&pool).await.expect("load edges");

        assert!(rows.is_empty());
    });

    cleanup_temp_db(&path);
}

#[test]
fn sqlite_avatar_profile_round_trips_as_json_setting() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let path = temp_db_path("avatar-profile-round-trip");

    runtime.block_on(async {
        let pool = open_pool(&path).await.expect("open sqlite");
        initialize_schema(&pool).await.expect("schema");
        let mut profile = AvatarReactionProfile {
            model_name: "Operator Cat".to_string(),
            model_path: "/tmp/operator-cat.vrm".to_string(),
            ..AvatarReactionProfile::default()
        };
        profile.critical.motion = AvatarMotion::Bounce;

        save_avatar_profile(&pool, &profile)
            .await
            .expect("save avatar profile");
        let restored = load_avatar_profile(&pool)
            .await
            .expect("load avatar profile")
            .expect("stored avatar profile");

        assert_eq!(restored, profile);
    });

    cleanup_temp_db(&path);
}

#[test]
fn corrupt_avatar_profile_does_not_disable_storage_worker() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let path = temp_db_path("avatar-profile-corrupt");

    runtime.block_on(async {
        let pool = open_pool(&path).await.expect("open sqlite");
        initialize_schema(&pool).await.expect("schema");
        sqlx::query("INSERT INTO app_settings (key, value_json) VALUES (?1, ?2)")
            .bind(AVATAR_PROFILE_KEY)
            .bind("{not-valid-json")
            .execute(&pool)
            .await
            .expect("seed corrupt avatar profile");
        pool.close().await;
    });

    let (command_tx, command_rx) = mpsc::channel();
    let (control_tx, control_rx) = mpsc::channel();
    let (init_tx, init_rx) = mpsc::channel();
    let worker_path = path.clone();
    let worker =
        std::thread::spawn(move || run_storage(worker_path, command_rx, control_rx, init_tx));

    let restored = init_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("storage worker init")
        .expect("corrupt profile must not disable storage");
    assert!(restored.avatar_profile.is_none());
    assert!(
        restored
            .avatar_profile_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("failed to decode"))
    );

    let profile = AvatarReactionProfile {
        model_name: "Recovered Cat".to_string(),
        ..AvatarReactionProfile::default()
    };
    let (result_tx, result_rx) = mpsc::channel();
    control_tx
        .send(StorageControlCommand::SaveAvatarProfile {
            profile: profile.clone(),
            result_tx,
        })
        .expect("save command");
    result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("save result")
        .expect("save succeeds");
    drop(command_tx);
    drop(control_tx);
    worker.join().expect("storage worker");

    runtime.block_on(async {
        let pool = open_pool(&path).await.expect("reopen sqlite");
        let restored = load_avatar_profile(&pool)
            .await
            .expect("load repaired profile")
            .expect("repaired avatar profile");
        assert_eq!(restored, profile);
    });

    cleanup_temp_db(&path);
}

#[test]
fn shutdown_ack_flushes_a_pending_profile_ahead_of_telemetry_backlog() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let path = temp_db_path("avatar-profile-shutdown");
    let (command_tx, command_rx) = mpsc::channel();
    let (control_tx, control_rx) = mpsc::channel();
    let (init_tx, init_rx) = mpsc::channel();
    let worker_path = path.clone();
    let worker =
        std::thread::spawn(move || run_storage(worker_path, command_rx, control_rx, init_tx));
    init_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("storage worker init")
        .expect("storage worker ready");

    for index in 0..64 {
        command_tx
            .send(StorageCommand::UpsertEdge(PersistedEdgeState {
                key: format!("edge-{index}"),
                endpoint: "127.0.0.1:9000".to_string(),
                node: format!("edge-{index}"),
                source: "test".to_string(),
                service: "skid-monitor-agent".to_string(),
                metric_points: index,
                spans: 0,
                log_records: 0,
                last_signal: "metrics".to_string(),
                last_metric: "system.cpu.usage".to_string(),
                last_value: "1".to_string(),
                last_seen_unix_ms: unix_millis(),
                severity: None,
            }))
            .expect("queue telemetry write");
    }

    let profile = AvatarReactionProfile {
        model_name: "Shutdown Safe Cat".to_string(),
        ..AvatarReactionProfile::default()
    };
    let (result_tx, result_rx) = mpsc::channel();
    control_tx
        .send(StorageControlCommand::SaveAvatarProfile {
            profile: profile.clone(),
            result_tx,
        })
        .expect("queue profile save");
    let (done_tx, done_rx) = mpsc::channel();
    control_tx
        .send(StorageControlCommand::Shutdown { done_tx })
        .expect("queue shutdown");

    result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("profile save result")
        .expect("profile save succeeds");
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("shutdown acknowledgement");
    drop(command_tx);
    drop(control_tx);
    worker.join().expect("storage worker");

    runtime.block_on(async {
        let pool = open_pool(&path).await.expect("reopen sqlite");
        let restored = load_avatar_profile(&pool)
            .await
            .expect("load profile")
            .expect("stored profile");
        assert_eq!(restored, profile);
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
