#[cfg(not(target_arch = "wasm32"))]
mod command;
#[cfg(not(target_arch = "wasm32"))]
mod db;
#[cfg(not(target_arch = "wasm32"))]
mod path;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(not(target_arch = "wasm32"))]
mod worker;

#[cfg(not(target_arch = "wasm32"))]
use command::StorageCommand;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

use crate::edge::PersistedEdgeState;
use crate::model::{AlertChange, AlertSeverity, AlertStatus, AlertTransition};
#[cfg(target_arch = "wasm32")]
use crate::platform::BrowserStorageScope;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use web_time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
const STORAGE_INIT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct StorageInit {
    pub(crate) storage: Option<StateStorage>,
    pub(crate) restored_edges: Vec<PersistedEdgeState>,
    pub(crate) message: Option<String>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) browser_scope: BrowserStorageScope,
}

#[derive(Clone)]
pub(crate) struct StateStorage {
    #[cfg(not(target_arch = "wasm32"))]
    tx: Sender<StorageCommand>,
    #[cfg(target_arch = "wasm32")]
    browser: web::BrowserStorage,
}

impl StateStorage {
    #[cfg(not(target_arch = "wasm32"))]
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

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start() -> StorageInit {
        let browser_scope = web::BrowserStorage::initial_scope();
        match web::BrowserStorage::open(browser_scope.clone()) {
            Ok((browser, restored_edges)) => StorageInit {
                storage: Some(Self { browser }),
                restored_edges,
                message: Some("browser state storage ready".to_string()),
                browser_scope,
            },
            Err(error) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                message: Some(format!("browser state storage disabled: {error}")),
                browser_scope,
            },
        }
    }

    pub(crate) fn persist_edge(&self, edge: &PersistedEdgeState) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.tx.send(StorageCommand::UpsertEdge(edge.clone()));
        #[cfg(target_arch = "wasm32")]
        self.browser.persist_edge(edge);
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn activate_browser_scope(
        &self,
        scope: BrowserStorageScope,
    ) -> Result<Vec<PersistedEdgeState>, String> {
        self.browser.activate_scope(scope)
    }

    pub(crate) fn delete_edge(&self, key: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.tx.send(StorageCommand::DeleteEdge(key.to_string()));
        #[cfg(target_arch = "wasm32")]
        self.browser.delete_edge(key);
    }

    pub(crate) fn persist_alert(&self, change: &AlertChange) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self
            .tx
            .send(StorageCommand::RecordAlert(AlertRecord::from(change)));
        #[cfg(target_arch = "wasm32")]
        self.browser.persist_alert(&AlertRecord::from(change));
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
