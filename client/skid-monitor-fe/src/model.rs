use skid_protocol::protocol::Signal;
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

pub(crate) enum ReceiverMessage {
    Listening(String),
    Signal(Signal),
    Error(String),
    ExtensionError(String),
}

pub(crate) struct EventRow {
    pub(crate) at: Instant,
    pub(crate) kind: String,
    pub(crate) message: String,
}

pub(crate) struct MetricSample {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) numeric: Option<f64>,
    pub(crate) source: String,
    pub(crate) kind: String,
    pub(crate) attributes: String,
    pub(crate) trend_key: String,
}
