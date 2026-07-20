use super::{AlertRecord, BrowserScopeRestore};
use crate::edge::PersistedEdgeState;
use crate::model::{AlertSeverity, AlertStatus, AlertTransition, AvatarReactionProfile};
use crate::platform::BrowserStorageScope;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use web_sys::{Storage, UrlSearchParams};

const EDGES_KEY: &str = "skid-monitor.edge-state.v1";
const ALERT_STATE_KEY: &str = "skid-monitor.alert-state.v1";
const ALERT_EVENTS_KEY: &str = "skid-monitor.alert-events.v1";
const AVATAR_PROFILE_KEY: &str = "skid-monitor.avatar-reaction-profile.v1";
const MAX_EDGE_STATES: usize = 512;
const MAX_ALERT_EVENTS: usize = 512;

#[derive(Clone)]
pub(super) struct BrowserStorage {
    storage: Storage,
    scope: Rc<RefCell<BrowserStorageScope>>,
}

#[derive(Clone, Deserialize, Serialize)]
struct BrowserAlertRecord {
    at_unix_ms: i64,
    transition: String,
    key: String,
    rule_id: String,
    severity: String,
    status: String,
    endpoint: String,
    node: String,
    source: String,
    summary: String,
    detail: String,
}

impl BrowserStorage {
    pub(super) fn initial_scope() -> BrowserStorageScope {
        let has_cloud_startup = web_sys::window()
            .and_then(|window| window.location().search().ok())
            .and_then(|search| UrlSearchParams::new_with_str(&search).ok())
            .and_then(|params| params.get("client_api"))
            .is_some_and(|value| !value.trim().is_empty());
        if has_cloud_startup {
            BrowserStorageScope::CloudPending
        } else {
            BrowserStorageScope::Legacy
        }
    }

    pub(super) fn open(
        initial_scope: BrowserStorageScope,
    ) -> Result<(Self, BrowserScopeRestore), String> {
        let storage = web_sys::window()
            .ok_or_else(|| "window is unavailable".to_string())?
            .local_storage()
            .map_err(js_error)?
            .ok_or_else(|| "localStorage is unavailable".to_string())?;
        let restored = restore_scope(&storage, &initial_scope);
        Ok((
            Self {
                storage,
                scope: Rc::new(RefCell::new(initial_scope)),
            },
            restored,
        ))
    }

    pub(super) fn activate_scope(
        &self,
        scope: BrowserStorageScope,
    ) -> Result<BrowserScopeRestore, String> {
        self.scope.replace(scope.clone());
        Ok(restore_scope(&self.storage, &scope))
    }

    pub(super) fn persist_edge(&self, edge: &PersistedEdgeState) {
        let Some(key) = self.storage_key(EDGES_KEY) else {
            return;
        };
        let result = (|| {
            let mut edges: Vec<PersistedEdgeState> =
                read_json(&self.storage, &key)?.unwrap_or_default();
            if let Some(existing) = edges.iter_mut().find(|existing| existing.key == edge.key) {
                existing.clone_from(edge);
            } else {
                edges.push(edge.clone());
            }
            edges.sort_by_key(|edge| std::cmp::Reverse(edge.last_seen_unix_ms));
            edges.truncate(MAX_EDGE_STATES);
            write_json(&self.storage, &key, &edges)
        })();
        report_error(result);
    }

    pub(super) fn delete_edge(&self, key: &str) {
        let Some(storage_key) = self.storage_key(EDGES_KEY) else {
            return;
        };
        let result = (|| {
            let mut edges: Vec<PersistedEdgeState> =
                read_json(&self.storage, &storage_key)?.unwrap_or_default();
            edges.retain(|edge| edge.key != key);
            write_json(&self.storage, &storage_key, &edges)
        })();
        report_error(result);
    }

    pub(super) fn persist_alert(&self, alert: &AlertRecord) {
        let Some(events_key) = self.storage_key(ALERT_EVENTS_KEY) else {
            return;
        };
        let Some(state_key) = self.storage_key(ALERT_STATE_KEY) else {
            return;
        };
        let result = (|| {
            let alert = BrowserAlertRecord::from(alert);
            let mut events: Vec<BrowserAlertRecord> =
                read_json(&self.storage, &events_key)?.unwrap_or_default();
            events.push(alert.clone());
            if events.len() > MAX_ALERT_EVENTS {
                events.drain(..events.len() - MAX_ALERT_EVENTS);
            }
            write_json(&self.storage, &events_key, &events)?;

            let mut active: BTreeMap<String, BrowserAlertRecord> =
                read_json(&self.storage, &state_key)?.unwrap_or_default();
            match alert.status.as_str() {
                "firing" => {
                    active.insert(alert.key.clone(), alert);
                }
                "resolved" => {
                    active.remove(&alert.key);
                }
                _ => {}
            }
            write_json(&self.storage, &state_key, &active)
        })();
        report_error(result);
    }

    pub(super) fn persist_avatar_profile(
        &self,
        profile: &AvatarReactionProfile,
    ) -> Result<(), String> {
        let key = self.storage_key(AVATAR_PROFILE_KEY).ok_or_else(|| {
            "character profile storage is unavailable until cloud authentication completes"
                .to_string()
        })?;
        write_json(&self.storage, &key, profile)
    }

    fn storage_key(&self, base: &str) -> Option<String> {
        self.scope.borrow().storage_key(base)
    }
}

fn restore_scope(storage: &Storage, scope: &BrowserStorageScope) -> BrowserScopeRestore {
    let mut warnings = Vec::new();
    let restored_edges = match scope.storage_key(EDGES_KEY) {
        Some(key) => match read_json(storage, &key) {
            Ok(edges) => edges.unwrap_or_default(),
            Err(error) => {
                warnings.push(format!("edge state ignored: {error}"));
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let avatar_profile = match scope.storage_key(AVATAR_PROFILE_KEY) {
        Some(key) => match read_json::<AvatarReactionProfile>(storage, &key) {
            Ok(Some(profile)) => match profile.normalized() {
                Ok(profile) => Some(profile),
                Err(error) => {
                    warnings.push(format!("avatar reaction profile ignored: {error}"));
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                warnings.push(format!("avatar reaction profile ignored: {error}"));
                None
            }
        },
        None => None,
    };

    BrowserScopeRestore {
        restored_edges,
        avatar_profile,
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    }
}

impl From<&AlertRecord> for BrowserAlertRecord {
    fn from(alert: &AlertRecord) -> Self {
        Self {
            at_unix_ms: alert.at_unix_ms,
            transition: match alert.transition {
                AlertTransition::Fired => "fired",
                AlertTransition::Resolved => "resolved",
            }
            .to_string(),
            key: alert.key.clone(),
            rule_id: alert.rule_id.clone(),
            severity: match alert.severity {
                AlertSeverity::Warning => "warning",
                AlertSeverity::Critical => "critical",
            }
            .to_string(),
            status: match alert.status {
                AlertStatus::Firing => "firing",
                AlertStatus::Resolved => "resolved",
            }
            .to_string(),
            endpoint: alert.endpoint.clone(),
            node: alert.node.clone(),
            source: alert.source.clone(),
            summary: alert.summary.clone(),
            detail: alert.detail.clone(),
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(
    storage: &Storage,
    key: &str,
) -> Result<Option<T>, String> {
    let Some(value) = storage.get_item(key).map_err(js_error)? else {
        return Ok(None);
    };
    serde_json::from_str(&value)
        .map(Some)
        .map_err(|error| format!("failed to decode {key}: {error}"))
}

fn write_json<T: Serialize>(storage: &Storage, key: &str, value: &T) -> Result<(), String> {
    let value =
        serde_json::to_string(value).map_err(|error| format!("failed to encode {key}: {error}"))?;
    storage.set_item(key, &value).map_err(js_error)
}

fn js_error(error: wasm_bindgen::JsValue) -> String {
    format!("{error:?}")
}

fn report_error(result: Result<(), String>) {
    if let Err(error) = result {
        web_sys::console::error_1(&format!("skid-monitor-fe storage error: {error}").into());
    }
}
