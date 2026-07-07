mod accessors;
mod events;
mod ingest;
mod lifecycle;
mod persistence;

use crate::alert::AlertStore;
use crate::edge::EdgeSignalDecorations;
use crate::model::{EventRow, MetricSample, NodeSummary, SignalCounters, Status};
use crate::storage::StateStorage;
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct DashboardState {
    pub(in crate::state) status: Status,
    pub(in crate::state) listening_label: Option<String>,
    pub(in crate::state) counters: SignalCounters,
    pub(in crate::state) events: VecDeque<EventRow>,
    pub(in crate::state) metrics: VecDeque<MetricSample>,
    pub(in crate::state) metric_history: BTreeMap<String, VecDeque<f64>>,
    pub(in crate::state) nodes: BTreeMap<String, NodeSummary>,
    pub(in crate::state) edge_decorations: EdgeSignalDecorations,
    pub(in crate::state) alerts: AlertStore,
    pub(in crate::state) alerts_enabled: bool,
    pub(in crate::state) storage: Option<StateStorage>,
}
