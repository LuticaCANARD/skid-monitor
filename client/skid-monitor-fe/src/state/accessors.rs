use super::DashboardState;
use crate::alert::AlertStore;
use crate::edge::EdgeSignalDecorations;
use crate::model::{AlertSummary, EventRow, MetricSample, NodeSummary, SignalCounters, Status};
use std::collections::{BTreeMap, VecDeque};

impl DashboardState {
    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    pub(crate) fn counters(&self) -> &SignalCounters {
        &self.counters
    }

    pub(crate) fn events(&self) -> &VecDeque<EventRow> {
        &self.events
    }

    pub(crate) fn metrics(&self) -> &VecDeque<MetricSample> {
        &self.metrics
    }

    pub(crate) fn metric_history(&self) -> &BTreeMap<String, VecDeque<f64>> {
        &self.metric_history
    }

    pub(crate) fn nodes(&self) -> &BTreeMap<String, NodeSummary> {
        &self.nodes
    }

    pub(crate) fn edge_decorations(&self) -> &EdgeSignalDecorations {
        &self.edge_decorations
    }

    pub(crate) fn alerts(&self) -> &AlertStore {
        &self.alerts
    }

    pub(crate) fn alerts_enabled(&self) -> bool {
        self.alerts_enabled
    }

    pub(crate) fn set_alerts_enabled(&mut self, enabled: bool) {
        if self.alerts_enabled == enabled {
            return;
        }

        self.alerts_enabled = enabled;
        if enabled {
            self.push_event("settings", "alerts enabled");
        } else {
            self.alerts.clear();
            let cleared_edges = self.edge_decorations.clear_severities();
            for edge in cleared_edges {
                self.persist_edge(edge);
            }
            self.push_event("settings", "alerts disabled");
        }
    }

    pub(crate) fn alert_summary(&self) -> AlertSummary {
        let mut summary = self.alerts.summary();
        summary.enabled = self.alerts_enabled;
        if !self.alerts_enabled {
            summary.active_count = 0;
            summary.highest_severity = None;
        }
        summary
    }
}
