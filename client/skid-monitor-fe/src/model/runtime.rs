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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OperationalSummary {
    pub(crate) agents: usize,
    pub(crate) listeners: usize,
    pub(crate) online: usize,
    pub(crate) pending: usize,
    pub(crate) warning: usize,
    pub(crate) critical: usize,
    pub(crate) storage_enabled: bool,
}
