use std::time::Instant;

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
