use super::AlertRecord;
use crate::edge::PersistedEdgeState;
use crate::model::{AlertSeverity, AlertStatus, AlertTransition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use web_sys::Storage;

const EDGES_KEY: &str = "skid-monitor.edge-state.v1";
const ALERT_STATE_KEY: &str = "skid-monitor.alert-state.v1";
const ALERT_EVENTS_KEY: &str = "skid-monitor.alert-events.v1";
const MAX_EDGE_STATES: usize = 512;
const MAX_ALERT_EVENTS: usize = 512;

#[derive(Clone)]
pub(super) struct BrowserStorage {
    storage: Storage,
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
    pub(super) fn open() -> Result<(Self, Vec<PersistedEdgeState>), String> {
        let storage = web_sys::window()
            .ok_or_else(|| "window is unavailable".to_string())?
            .local_storage()
            .map_err(js_error)?
            .ok_or_else(|| "localStorage is unavailable".to_string())?;
        let restored_edges = read_json(&storage, EDGES_KEY)?.unwrap_or_default();
        Ok((Self { storage }, restored_edges))
    }

    pub(super) fn persist_edge(&self, edge: &PersistedEdgeState) {
        let result = (|| {
            let mut edges: Vec<PersistedEdgeState> =
                read_json(&self.storage, EDGES_KEY)?.unwrap_or_default();
            if let Some(existing) = edges.iter_mut().find(|existing| existing.key == edge.key) {
                existing.clone_from(edge);
            } else {
                edges.push(edge.clone());
            }
            edges.sort_by_key(|edge| std::cmp::Reverse(edge.last_seen_unix_ms));
            edges.truncate(MAX_EDGE_STATES);
            write_json(&self.storage, EDGES_KEY, &edges)
        })();
        report_error(result);
    }

    pub(super) fn delete_edge(&self, key: &str) {
        let result = (|| {
            let mut edges: Vec<PersistedEdgeState> =
                read_json(&self.storage, EDGES_KEY)?.unwrap_or_default();
            edges.retain(|edge| edge.key != key);
            write_json(&self.storage, EDGES_KEY, &edges)
        })();
        report_error(result);
    }

    pub(super) fn persist_alert(&self, alert: &AlertRecord) {
        let result = (|| {
            let alert = BrowserAlertRecord::from(alert);
            let mut events: Vec<BrowserAlertRecord> =
                read_json(&self.storage, ALERT_EVENTS_KEY)?.unwrap_or_default();
            events.push(alert.clone());
            if events.len() > MAX_ALERT_EVENTS {
                events.drain(..events.len() - MAX_ALERT_EVENTS);
            }
            write_json(&self.storage, ALERT_EVENTS_KEY, &events)?;

            let mut active: BTreeMap<String, BrowserAlertRecord> =
                read_json(&self.storage, ALERT_STATE_KEY)?.unwrap_or_default();
            match alert.status.as_str() {
                "firing" => {
                    active.insert(alert.key.clone(), alert);
                }
                "resolved" => {
                    active.remove(&alert.key);
                }
                _ => {}
            }
            write_json(&self.storage, ALERT_STATE_KEY, &active)
        })();
        report_error(result);
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
