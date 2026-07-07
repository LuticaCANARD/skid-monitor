use std::time::Instant;

#[derive(Default)]
pub(crate) struct SignalCounters {
    pub(crate) metrics: usize,
    pub(crate) metric_points: usize,
    pub(crate) traces: usize,
    pub(crate) spans: usize,
    pub(crate) logs: usize,
    pub(crate) log_records: usize,
}

pub(crate) enum Status {
    Starting,
    Listening(String),
    Error(String),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum AlertSeverity {
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AlertStatus {
    Firing,
    Resolved,
}

#[derive(Clone, Debug)]
pub(crate) struct AlertSnapshot {
    pub(crate) key: String,
    pub(crate) rule_id: String,
    pub(crate) severity: AlertSeverity,
    pub(crate) status: AlertStatus,
    pub(crate) endpoint: String,
    pub(crate) node: String,
    pub(crate) source: String,
    pub(crate) summary: String,
    pub(crate) detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AlertTransition {
    Fired,
    Resolved,
}

pub(crate) struct AlertChange {
    pub(crate) transition: AlertTransition,
    pub(crate) snapshot: AlertSnapshot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AlertSummary {
    pub(crate) active_count: usize,
    pub(crate) highest_severity: Option<AlertSeverity>,
}

#[derive(Clone)]
pub(crate) struct EventRow {
    pub(crate) time: String,
    pub(crate) kind: String,
    pub(crate) message: String,
}

#[derive(Clone)]
pub(crate) struct NodeSummary {
    pub(crate) node: String,
    pub(crate) endpoint: String,
    pub(crate) source: String,
    pub(crate) service: String,
    pub(crate) metric_points: usize,
    pub(crate) spans: usize,
    pub(crate) log_records: usize,
    pub(crate) last_metric: String,
    pub(crate) last_value: String,
    pub(crate) last_seen: Instant,
}

#[derive(Clone)]
pub(crate) struct MetricSample {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) numeric: Option<f64>,
    pub(crate) source: String,
    pub(crate) service: String,
    pub(crate) node: String,
    pub(crate) endpoint: String,
    pub(crate) kind: String,
    pub(crate) attributes: String,
    pub(crate) trend_key: String,
}
