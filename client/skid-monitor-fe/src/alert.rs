use crate::config;
use crate::model::{AlertChange, AlertSeverity, AlertSnapshot, AlertStatus, AlertSummary};
use crate::model::{AlertTransition, MetricSample};
use crate::utils::format_f64;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

#[derive(Default)]
pub(crate) struct AlertStore {
    active: BTreeMap<String, AlertSnapshot>,
}

impl AlertStore {
    pub(crate) fn observe_receiver_error(
        &mut self,
        listener: &str,
        error: &str,
    ) -> Option<AlertChange> {
        self.fire(
            format!("receiver.error:{listener}"),
            "receiver.error",
            AlertSeverity::Critical,
            listener,
            listener,
            listener,
            "Receiver error",
            error.to_string(),
        )
    }

    pub(crate) fn observe_receiver_recovered(
        &mut self,
        listener: &str,
        detail: &str,
    ) -> Option<AlertChange> {
        self.resolve(&format!("receiver.error:{listener}"), detail.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn observe_extension_error(&mut self, error: &str) -> Option<AlertChange> {
        self.fire(
            "extension.error".to_string(),
            "extension.error",
            AlertSeverity::Warning,
            "frontend",
            "extension-host",
            "frontend",
            "Extension host error",
            error.to_string(),
        )
    }

    pub(crate) fn observe_metric(&mut self, sample: &MetricSample) -> Option<AlertChange> {
        let evaluation = MetricEvaluation::from_sample(sample)?;
        if evaluation.firing {
            self.fire(
                evaluation.key,
                evaluation.rule_id,
                evaluation.severity,
                &evaluation.endpoint,
                &evaluation.node,
                &evaluation.source,
                evaluation.summary,
                evaluation.detail,
            )
        } else {
            self.resolve(&evaluation.key, evaluation.detail)
        }
    }

    pub(crate) fn summary(&self) -> AlertSummary {
        AlertSummary {
            enabled: true,
            active_count: self.active.len(),
            highest_severity: self.active.values().map(|alert| alert.severity).max(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.active.clear();
    }

    pub(crate) fn active_for_metric(&self, sample: &MetricSample) -> Option<AlertSeverity> {
        metric_alert_key(sample).and_then(|key| self.active.get(&key).map(|alert| alert.severity))
    }

    pub(crate) fn highest_for_node(&self, endpoint: &str, node: &str) -> Option<AlertSeverity> {
        self.active
            .values()
            .filter(|alert| alert.endpoint == endpoint && alert.node == node)
            .map(|alert| alert.severity)
            .max()
    }

    pub(crate) fn highest_for_presenter(
        &self,
        endpoint: &str,
        node: &str,
    ) -> Option<AlertSeverity> {
        self.active
            .values()
            .filter(|alert| alert_relevant_to_presenter(alert, endpoint, node))
            .map(|alert| alert.severity)
            .max()
    }

    pub(crate) fn active_count_for_presenter(&self, endpoint: &str, node: &str) -> usize {
        self.active
            .values()
            .filter(|alert| alert_relevant_to_presenter(alert, endpoint, node))
            .count()
    }

    fn fire(
        &mut self,
        key: String,
        rule_id: &str,
        severity: AlertSeverity,
        endpoint: &str,
        node: &str,
        source: &str,
        summary: &str,
        detail: String,
    ) -> Option<AlertChange> {
        let snapshot = AlertSnapshot {
            key,
            rule_id: rule_id.to_string(),
            severity,
            status: AlertStatus::Firing,
            endpoint: endpoint.to_string(),
            node: node.to_string(),
            source: source.to_string(),
            summary: summary.to_string(),
            detail,
        };

        match self.active.entry(snapshot.key.clone()) {
            Entry::Occupied(mut entry) => {
                entry.insert(snapshot);
                None
            }
            Entry::Vacant(entry) => {
                entry.insert(snapshot.clone());
                Some(AlertChange {
                    transition: AlertTransition::Fired,
                    snapshot,
                })
            }
        }
    }

    fn resolve(&mut self, key: &str, detail: String) -> Option<AlertChange> {
        let mut snapshot = self.active.remove(key)?;
        snapshot.status = AlertStatus::Resolved;
        snapshot.detail = detail;

        Some(AlertChange {
            transition: AlertTransition::Resolved,
            snapshot,
        })
    }
}

fn alert_relevant_to_presenter(alert: &AlertSnapshot, endpoint: &str, node: &str) -> bool {
    alert.endpoint == endpoint && (alert.node == node || alert.rule_id == "receiver.error")
}

struct MetricEvaluation {
    key: String,
    rule_id: &'static str,
    severity: AlertSeverity,
    source: String,
    endpoint: String,
    node: String,
    summary: &'static str,
    detail: String,
    firing: bool,
}

impl MetricEvaluation {
    fn from_sample(sample: &MetricSample) -> Option<Self> {
        let value = sample.numeric?;
        match sample.name.as_str() {
            "system.cpu.usage" => Some(threshold_evaluation(
                sample,
                value,
                "system.cpu.high",
                AlertSeverity::Warning,
                "High CPU usage",
                config::ALERT_CPU_USAGE_WARNING_THRESHOLD,
            )),
            "system.memory.usage" => Some(threshold_evaluation(
                sample,
                value,
                "system.memory.high",
                AlertSeverity::Warning,
                "High memory usage",
                config::ALERT_MEMORY_USAGE_WARNING_THRESHOLD,
            )),
            "file_node.root.available" => Some(file_root_available_evaluation(sample, value)),
            _ => None,
        }
    }
}

fn threshold_evaluation(
    sample: &MetricSample,
    value: f64,
    rule_id: &'static str,
    severity: AlertSeverity,
    summary: &'static str,
    threshold: f64,
) -> MetricEvaluation {
    let firing = value >= threshold;
    let detail = if firing {
        format!(
            "{} reported {} for {} (threshold {}, {}).",
            sample.source,
            sample.value,
            sample.name,
            format_f64(threshold),
            sample.attributes
        )
    } else {
        format!(
            "{} recovered: {} is {} (threshold {}).",
            sample.source,
            sample.name,
            sample.value,
            format_f64(threshold)
        )
    };

    MetricEvaluation {
        key: rule_metric_key(rule_id, sample),
        rule_id,
        severity,
        source: sample.source.clone(),
        endpoint: sample.endpoint.clone(),
        node: sample.node.clone(),
        summary,
        detail,
        firing,
    }
}

fn file_root_available_evaluation(sample: &MetricSample, value: f64) -> MetricEvaluation {
    let firing = value <= 0.0;
    let detail = if firing {
        format!(
            "{} reported an unavailable file root ({}).",
            sample.source, sample.attributes
        )
    } else {
        format!("{} file root is available again.", sample.source)
    };

    MetricEvaluation {
        key: rule_metric_key("file.root.unavailable", sample),
        rule_id: "file.root.unavailable",
        severity: AlertSeverity::Critical,
        source: sample.source.clone(),
        endpoint: sample.endpoint.clone(),
        node: sample.node.clone(),
        summary: "File root unavailable",
        detail,
        firing,
    }
}

fn metric_alert_key(sample: &MetricSample) -> Option<String> {
    let rule_id = match sample.name.as_str() {
        "system.cpu.usage" => "system.cpu.high",
        "system.memory.usage" => "system.memory.high",
        "file_node.root.available" => "file.root.unavailable",
        _ => return None,
    };
    Some(rule_metric_key(rule_id, sample))
}

fn rule_metric_key(rule_id: &str, sample: &MetricSample) -> String {
    format!("{rule_id}:{}:{}", sample.endpoint, sample.trend_key)
}

#[cfg(test)]
mod tests;
