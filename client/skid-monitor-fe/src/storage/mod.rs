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
use command::{StorageCommand, StorageControlCommand};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

use crate::edge::PersistedEdgeState;
use crate::model::{
    AlertChange, AlertSeverity, AlertStatus, AlertTransition, AvatarReactionProfile,
};
#[cfg(target_arch = "wasm32")]
use crate::platform::BrowserStorageScope;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::{self, JoinHandle};
use web_time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
const STORAGE_INIT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(target_arch = "wasm32"))]
const STORAGE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type AvatarProfileSaveReceiver = Receiver<Result<(), String>>;

pub(crate) struct StorageInit {
    pub(crate) storage: Option<StateStorage>,
    pub(crate) restored_edges: Vec<PersistedEdgeState>,
    pub(crate) avatar_profile: Option<AvatarReactionProfile>,
    pub(crate) message: Option<String>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) browser_scope: BrowserStorageScope,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeStorageRestore {
    restored_edges: Vec<PersistedEdgeState>,
    avatar_profile: Option<AvatarReactionProfile>,
    avatar_profile_warning: Option<String>,
}

#[cfg(target_arch = "wasm32")]
pub(super) struct BrowserScopeRestore {
    pub(super) restored_edges: Vec<PersistedEdgeState>,
    pub(super) avatar_profile: Option<AvatarReactionProfile>,
    pub(super) warning: Option<String>,
}

pub(crate) struct StateStorage {
    #[cfg(not(target_arch = "wasm32"))]
    tx: Option<Sender<StorageCommand>>,
    #[cfg(not(target_arch = "wasm32"))]
    control_tx: Option<Sender<StorageControlCommand>>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<JoinHandle<()>>,
    #[cfg(target_arch = "wasm32")]
    browser: web::BrowserStorage,
}

impl StateStorage {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start() -> StorageInit {
        Self::start_at(path::state_db_path())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start_at(path: PathBuf) -> StorageInit {
        let label = path.display().to_string();
        let (tx, rx) = mpsc::channel();
        let (control_tx, control_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::channel();
        let worker = thread::spawn(move || worker::run_storage(path, rx, control_rx, init_tx));

        match init_rx.recv_timeout(STORAGE_INIT_TIMEOUT) {
            Ok(Ok(restored)) => {
                let message = match restored.avatar_profile_warning {
                    Some(warning) => format!("state db ready at {label}; {warning}"),
                    None => format!("state db ready at {label}"),
                };
                StorageInit {
                    storage: Some(Self {
                        tx: Some(tx),
                        control_tx: Some(control_tx),
                        worker: Some(worker),
                    }),
                    restored_edges: restored.restored_edges,
                    avatar_profile: restored.avatar_profile,
                    message: Some(message),
                }
            }
            Ok(Err(error)) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                avatar_profile: None,
                message: Some(format!("state db disabled: {error}")),
            },
            Err(error) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                avatar_profile: None,
                message: Some(format!("state db startup timed out: {error}")),
            },
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start() -> StorageInit {
        let browser_scope = web::BrowserStorage::initial_scope();
        match web::BrowserStorage::open(browser_scope.clone()) {
            Ok((browser, restored)) => {
                let message = match restored.warning {
                    Some(warning) => format!("browser state storage ready; {warning}"),
                    None => "browser state storage ready".to_string(),
                };
                StorageInit {
                    storage: Some(Self { browser }),
                    restored_edges: restored.restored_edges,
                    avatar_profile: restored.avatar_profile,
                    message: Some(message),
                    browser_scope,
                }
            }
            Err(error) => StorageInit {
                storage: None,
                restored_edges: Vec::new(),
                avatar_profile: None,
                message: Some(format!("browser state storage disabled: {error}")),
                browser_scope,
            },
        }
    }

    pub(crate) fn persist_edge(&self, edge: &PersistedEdgeState) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &self.tx {
            let _ = tx.send(StorageCommand::UpsertEdge(edge.clone()));
        }
        #[cfg(target_arch = "wasm32")]
        self.browser.persist_edge(edge);
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn activate_browser_scope(
        &self,
        scope: BrowserStorageScope,
    ) -> Result<BrowserScopeRestore, String> {
        self.browser.activate_scope(scope)
    }

    pub(crate) fn delete_edge(&self, key: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &self.tx {
            let _ = tx.send(StorageCommand::DeleteEdge(key.to_string()));
        }
        #[cfg(target_arch = "wasm32")]
        self.browser.delete_edge(key);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn queue_avatar_profile_save(
        &self,
        profile: &AvatarReactionProfile,
    ) -> Result<AvatarProfileSaveReceiver, String> {
        let control_tx = self
            .control_tx
            .as_ref()
            .ok_or_else(|| "state storage worker is unavailable".to_string())?;
        let (result_tx, result_rx) = mpsc::channel();
        control_tx
            .send(StorageControlCommand::SaveAvatarProfile {
                profile: profile.clone(),
                result_tx,
            })
            .map_err(|error| format!("failed to queue character profile save: {error}"))?;
        Ok(result_rx)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn persist_avatar_profile(
        &self,
        profile: &AvatarReactionProfile,
    ) -> Result<(), String> {
        self.browser.persist_avatar_profile(profile)
    }

    pub(crate) fn persist_alert(&self, change: &AlertChange) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(tx) = &self.tx {
            let _ = tx.send(StorageCommand::RecordAlert(AlertRecord::from(change)));
        }
        #[cfg(target_arch = "wasm32")]
        self.browser.persist_alert(&AlertRecord::from(change));
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for StateStorage {
    fn drop(&mut self) {
        if let Some(control_tx) = self.control_tx.take() {
            let (done_tx, done_rx) = mpsc::channel();
            let _ = control_tx.send(StorageControlCommand::Shutdown { done_tx });
            let _ = done_rx.recv_timeout(STORAGE_SHUTDOWN_TIMEOUT);
        }
        self.tx.take();
        self.worker.take();
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
