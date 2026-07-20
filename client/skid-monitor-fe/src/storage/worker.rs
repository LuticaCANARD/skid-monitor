use super::{NativeStorageRestore, StorageCommand, StorageControlCommand, db};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn run_storage(
    path: PathBuf,
    rx: Receiver<StorageCommand>,
    control_rx: Receiver<StorageControlCommand>,
    init_tx: Sender<Result<NativeStorageRestore, String>>,
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

    let pool = match runtime.block_on(db::open_pool(&path)) {
        Ok(pool) => pool,
        Err(error) => {
            let _ = init_tx.send(Err(error));
            return;
        }
    };

    if let Err(error) = runtime.block_on(db::initialize_schema(&pool)) {
        let _ = init_tx.send(Err(error.to_string()));
        return;
    }

    let restored_edges = match runtime.block_on(db::load_edge_states(&pool)) {
        Ok(edges) => edges,
        Err(error) => {
            let _ = init_tx.send(Err(error.to_string()));
            return;
        }
    };

    let (avatar_profile, avatar_profile_warning) =
        match runtime.block_on(db::load_avatar_profile(&pool)) {
            Ok(profile) => (profile, None),
            Err(error) => (
                None,
                Some(format!("avatar reaction profile ignored: {error}")),
            ),
        };

    if init_tx
        .send(Ok(NativeStorageRestore {
            restored_edges,
            avatar_profile,
            avatar_profile_warning,
        }))
        .is_err()
    {
        return;
    }

    let mut control_open = true;
    loop {
        if control_open {
            match control_rx.try_recv() {
                Ok(command) => match handle_control(&runtime, &pool, command) {
                    ControlFlow::Continue => continue,
                    ControlFlow::Shutdown(done_tx) => {
                        drain_telemetry(&runtime, &pool, &rx);
                        let _ = done_tx.send(());
                        break;
                    }
                },
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => control_open = false,
            }
        }

        let command = if control_open {
            match rx.recv_timeout(CONTROL_POLL_INTERVAL) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => match control_rx.recv() {
                    Ok(command) => match handle_control(&runtime, &pool, command) {
                        ControlFlow::Continue => continue,
                        ControlFlow::Shutdown(done_tx) => {
                            let _ = done_tx.send(());
                            break;
                        }
                    },
                    Err(_) => break,
                },
            }
        } else {
            let Ok(command) = rx.recv() else {
                break;
            };
            command
        };

        if let Err(error) = handle_telemetry(&runtime, &pool, command) {
            eprintln!("skid-monitor-fe state db write failed: {error}");
        }
    }
}

enum ControlFlow {
    Continue,
    Shutdown(Sender<()>),
}

fn handle_control(
    runtime: &tokio::runtime::Runtime,
    pool: &sqlx::sqlite::SqlitePool,
    command: StorageControlCommand,
) -> ControlFlow {
    match command {
        StorageControlCommand::SaveAvatarProfile { profile, result_tx } => {
            let result = runtime.block_on(db::save_avatar_profile(pool, &profile));
            if result_tx.send(result).is_err() {
                eprintln!("skid-monitor-fe character profile save result was not received");
            }
            ControlFlow::Continue
        }
        StorageControlCommand::Shutdown { done_tx } => ControlFlow::Shutdown(done_tx),
    }
}

fn handle_telemetry(
    runtime: &tokio::runtime::Runtime,
    pool: &sqlx::sqlite::SqlitePool,
    command: StorageCommand,
) -> Result<(), String> {
    match command {
        StorageCommand::UpsertEdge(edge) => runtime
            .block_on(db::upsert_edge_state(pool, &edge))
            .map_err(|error| error.to_string()),
        StorageCommand::DeleteEdge(key) => runtime
            .block_on(db::delete_edge_state(pool, &key))
            .map_err(|error| error.to_string()),
        StorageCommand::RecordAlert(alert) => runtime
            .block_on(db::record_alert(pool, &alert))
            .map_err(|error| error.to_string()),
    }
}

fn drain_telemetry(
    runtime: &tokio::runtime::Runtime,
    pool: &sqlx::sqlite::SqlitePool,
    rx: &Receiver<StorageCommand>,
) {
    while let Ok(command) = rx.try_recv() {
        if let Err(error) = handle_telemetry(runtime, pool, command) {
            eprintln!("skid-monitor-fe state db write failed during shutdown: {error}");
        }
    }
}
