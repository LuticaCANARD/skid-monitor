use crate::model::{AlertSeverity, NodeSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use web_time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PersistedEdgeState {
    pub(crate) key: String,
    pub(crate) endpoint: String,
    pub(crate) node: String,
    pub(crate) source: String,
    pub(crate) service: String,
    pub(crate) metric_points: i64,
    pub(crate) spans: i64,
    pub(crate) log_records: i64,
    pub(crate) last_signal: String,
    pub(crate) last_metric: String,
    pub(crate) last_value: String,
    pub(crate) last_seen_unix_ms: i64,
    pub(crate) severity: Option<AlertSeverity>,
}

#[derive(Clone, Debug)]
pub(crate) struct EdgeSignalDecoration {
    pub(crate) key: String,
    pub(crate) endpoint: String,
    pub(crate) node: String,
    pub(crate) source: String,
    pub(crate) service: String,
    pub(crate) metric_points: usize,
    pub(crate) spans: usize,
    pub(crate) log_records: usize,
    pub(crate) last_signal: String,
    pub(crate) last_metric: String,
    pub(crate) last_value: String,
    pub(crate) last_seen: Instant,
    pub(crate) last_seen_unix_ms: i64,
    pub(crate) severity: Option<AlertSeverity>,
}

#[derive(Default)]
pub(crate) struct EdgeSignalDecorations {
    rows: BTreeMap<String, EdgeSignalDecoration>,
}

impl EdgeSignalDecorations {
    pub(crate) fn get(&self, endpoint: &str, node: &str) -> Option<&EdgeSignalDecoration> {
        self.rows.get(&edge_key(endpoint, node))
    }

    pub(crate) fn restore(&mut self, states: Vec<PersistedEdgeState>) -> Vec<NodeSummary> {
        let mut nodes = Vec::with_capacity(states.len());
        for state in states {
            let decoration = EdgeSignalDecoration::from_persisted(state);
            nodes.push(decoration.to_node_summary());
            self.rows.insert(decoration.key.clone(), decoration);
        }
        nodes
    }

    pub(crate) fn observe_node(
        &mut self,
        node: &NodeSummary,
        last_signal: &str,
    ) -> PersistedEdgeState {
        let key = edge_key(&node.endpoint, &node.node);
        let last_seen_unix_ms = unix_millis();
        let decoration = self
            .rows
            .entry(key.clone())
            .or_insert_with(|| EdgeSignalDecoration {
                key,
                endpoint: node.endpoint.clone(),
                node: node.node.clone(),
                source: node.source.clone(),
                service: node.service.clone(),
                metric_points: 0,
                spans: 0,
                log_records: 0,
                last_signal: String::new(),
                last_metric: String::new(),
                last_value: String::new(),
                last_seen: Instant::now(),
                last_seen_unix_ms,
                severity: None,
            });

        decoration.endpoint = node.endpoint.clone();
        decoration.node = node.node.clone();
        decoration.source = node.source.clone();
        decoration.service = node.service.clone();
        decoration.metric_points = node.metric_points;
        decoration.spans = node.spans;
        decoration.log_records = node.log_records;
        decoration.last_signal = last_signal.to_string();
        decoration.last_metric = node.last_metric.clone();
        decoration.last_value = node.last_value.clone();
        decoration.last_seen = node.last_seen;
        decoration.last_seen_unix_ms = last_seen_unix_ms;
        decoration.to_persisted()
    }

    pub(crate) fn set_node_severity(
        &mut self,
        endpoint: &str,
        node: &str,
        severity: Option<AlertSeverity>,
    ) -> Option<PersistedEdgeState> {
        let decoration = self.rows.get_mut(&edge_key(endpoint, node))?;
        decoration.severity = severity;
        decoration.last_seen_unix_ms = unix_millis();
        Some(decoration.to_persisted())
    }

    pub(crate) fn remove(&mut self, key: &str) -> bool {
        self.rows.remove(key).is_some()
    }

    pub(crate) fn clear_severities(&mut self) -> Vec<PersistedEdgeState> {
        let mut cleared = Vec::new();
        for decoration in self.rows.values_mut() {
            if decoration.severity.is_some() {
                decoration.severity = None;
                decoration.last_seen_unix_ms = unix_millis();
                cleared.push(decoration.to_persisted());
            }
        }
        cleared
    }
}

impl EdgeSignalDecoration {
    fn from_persisted(state: PersistedEdgeState) -> Self {
        let last_seen = instant_from_unix_millis(state.last_seen_unix_ms);
        Self {
            key: state.key,
            endpoint: state.endpoint,
            node: state.node,
            source: state.source,
            service: state.service,
            metric_points: state.metric_points.max(0) as usize,
            spans: state.spans.max(0) as usize,
            log_records: state.log_records.max(0) as usize,
            last_signal: state.last_signal,
            last_metric: state.last_metric,
            last_value: state.last_value,
            last_seen,
            last_seen_unix_ms: state.last_seen_unix_ms,
            severity: state.severity,
        }
    }

    fn to_persisted(&self) -> PersistedEdgeState {
        PersistedEdgeState {
            key: self.key.clone(),
            endpoint: self.endpoint.clone(),
            node: self.node.clone(),
            source: self.source.clone(),
            service: self.service.clone(),
            metric_points: self.metric_points as i64,
            spans: self.spans as i64,
            log_records: self.log_records as i64,
            last_signal: self.last_signal.clone(),
            last_metric: self.last_metric.clone(),
            last_value: self.last_value.clone(),
            last_seen_unix_ms: self.last_seen_unix_ms,
            severity: self.severity,
        }
    }

    fn to_node_summary(&self) -> NodeSummary {
        NodeSummary {
            node: self.node.clone(),
            endpoint: self.endpoint.clone(),
            source: self.source.clone(),
            service: self.service.clone(),
            metric_points: self.metric_points,
            spans: self.spans,
            log_records: self.log_records,
            last_metric: self.last_metric.clone(),
            last_value: self.last_value.clone(),
            last_seen: self.last_seen,
        }
    }
}

pub(crate) fn edge_key(endpoint: &str, node: &str) -> String {
    format!("{endpoint}|{node}")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn severity_name(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Warning => "warning",
        AlertSeverity::Critical => "critical",
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn severity_from_name(value: &str) -> Option<AlertSeverity> {
    match value {
        "warning" => Some(AlertSeverity::Warning),
        "critical" => Some(AlertSeverity::Critical),
        _ => None,
    }
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis() as i64
}

fn instant_from_unix_millis(unix_ms: i64) -> Instant {
    let age_ms = unix_millis().saturating_sub(unix_ms).max(0) as u64;
    Instant::now()
        .checked_sub(Duration::from_millis(age_ms))
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod tests;
