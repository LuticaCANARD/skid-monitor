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

    pub(crate) fn alert_summary(&self) -> AlertSummary {
        self.alerts.summary()
    }
}
