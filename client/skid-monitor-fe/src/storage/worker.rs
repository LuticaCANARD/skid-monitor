use super::{StorageCommand, db};
use crate::edge::PersistedEdgeState;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

pub(super) fn run_storage(
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

    if init_tx.send(Ok(restored_edges)).is_err() {
        return;
    }

    while let Ok(command) = rx.recv() {
        let result = match command {
            StorageCommand::UpsertEdge(edge) => {
                runtime.block_on(db::upsert_edge_state(&pool, &edge))
            }
            StorageCommand::DeleteEdge(key) => runtime.block_on(db::delete_edge_state(&pool, &key)),
            StorageCommand::RecordAlert(alert) => runtime.block_on(db::record_alert(&pool, &alert)),
        };
        if let Err(error) = result {
            eprintln!("skid-monitor-fe state db write failed: {error}");
        }
    }
}
