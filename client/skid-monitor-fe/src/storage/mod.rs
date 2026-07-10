mod command;
mod db;
mod path;
mod worker;

use command::StorageCommand;

#[cfg(test)]
mod tests;

use crate::edge::PersistedEdgeState;
use crate::model::{AlertChange, AlertSeverity, AlertStatus, AlertTransition};
use std::sync::mpsc::{self, Sender};
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
        let path = path::state_db_path();
        let label = path.display().to_string();
        let (tx, rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::channel();
        thread::spawn(move || worker::run_storage(path, rx, init_tx));

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

    pub(crate) fn delete_edge(&self, key: &str) {
        let _ = self.tx.send(StorageCommand::DeleteEdge(key.to_string()));
    }

    pub(crate) fn persist_alert(&self, change: &AlertChange) {
        let _ = self
            .tx
            .send(StorageCommand::RecordAlert(AlertRecord::from(change)));
    }
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

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis() as i64
}
