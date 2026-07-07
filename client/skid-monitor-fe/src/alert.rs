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
    pub(crate) fn observe_receiver_error(&mut self, error: &str) -> Option<AlertChange> {
        self.fire(
            "receiver.error".to_string(),
            "receiver.error",
            AlertSeverity::Critical,
            "frontend",
            "Receiver error",
            error.to_string(),
        )
    }

    pub(crate) fn observe_receiver_recovered(&mut self, detail: &str) -> Option<AlertChange> {
        self.resolve("receiver.error", detail.to_string())
    }

    pub(crate) fn observe_extension_error(&mut self, error: &str) -> Option<AlertChange> {
        self.fire(
            "extension.error".to_string(),
            "extension.error",
            AlertSeverity::Warning,
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
            active_count: self.active.len(),
            highest_severity: self.active.values().map(|alert| alert.severity).max(),
        }
    }

    pub(crate) fn active_for_metric(&self, sample: &MetricSample) -> Option<AlertSeverity> {
        metric_alert_key(sample).and_then(|key| self.active.get(&key).map(|alert| alert.severity))
    }

    fn fire(
        &mut self,
        key: String,
        rule_id: &str,
        severity: AlertSeverity,
        source: &str,
        summary: &str,
        detail: String,
    ) -> Option<AlertChange> {
        let snapshot = AlertSnapshot {
            key,
            rule_id: rule_id.to_string(),
            severity,
            status: AlertStatus::Firing,
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

struct MetricEvaluation {
    key: String,
    rule_id: &'static str,
    severity: AlertSeverity,
    source: String,
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
    format!("{rule_id}:{}", sample.trend_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_alert_fires_once_until_resolved() {
        let mut alerts = AlertStore::default();
        let hot = sample("system.cpu.usage", 95.0, "95");

        let first = alerts.observe_metric(&hot).expect("first alert");
        assert_eq!(first.transition, AlertTransition::Fired);
        assert_eq!(alerts.summary().active_count, 1);

        assert!(alerts.observe_metric(&hot).is_none());
        assert_eq!(alerts.summary().active_count, 1);

        let cool = sample("system.cpu.usage", 35.0, "35");
        let resolved = alerts.observe_metric(&cool).expect("resolved alert");
        assert_eq!(resolved.transition, AlertTransition::Resolved);
        assert_eq!(alerts.summary().active_count, 0);
    }

    #[test]
    fn file_root_unavailable_is_critical() {
        let mut alerts = AlertStore::default();
        let unavailable = sample("file_node.root.available", 0.0, "0");

        let change = alerts.observe_metric(&unavailable).expect("critical alert");

        assert_eq!(change.snapshot.severity, AlertSeverity::Critical);
        assert_eq!(
            alerts.summary().highest_severity,
            Some(AlertSeverity::Critical)
        );
    }

    #[test]
    fn unknown_metric_does_not_alert() {
        let mut alerts = AlertStore::default();
        assert!(
            alerts
                .observe_metric(&sample("custom.metric", 100.0, "100"))
                .is_none()
        );
    }

    fn sample(name: &str, numeric: f64, value: &str) -> MetricSample {
        MetricSample {
            name: name.to_string(),
            value: value.to_string(),
            numeric: Some(numeric),
            source: "agent".to_string(),
            service: "agent".to_string(),
            node: "agent@fixture".to_string(),
            endpoint: "fixture".to_string(),
            kind: "gauge".to_string(),
            attributes: "service=agent, scope=test".to_string(),
            trend_key: format!("agent/{name}"),
        }
    }
}
