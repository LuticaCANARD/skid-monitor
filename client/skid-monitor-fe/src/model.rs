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
    pub(crate) enabled: bool,
    pub(crate) active_count: usize,
    pub(crate) highest_severity: Option<AlertSeverity>,
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
    pub(crate) signal_subtype: MetricSignalSubtype,
    pub(crate) database_system: Option<DatabaseSystem>,
    pub(crate) database_namespace: String,
    pub(crate) database_operation: String,
    pub(crate) database_target: String,
    pub(crate) source: String,
    pub(crate) service: String,
    pub(crate) node: String,
    pub(crate) endpoint: String,
    pub(crate) kind: String,
    pub(crate) attributes: String,
    pub(crate) trend_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetricSignalSubtype {
    OpenTelemetry,
    Database,
}

impl MetricSignalSubtype {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OpenTelemetry => "opentelemetry",
            Self::Database => "database",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatabaseSystem {
    MySql,
    PostgreSql,
    Redis,
    Valkey,
}

impl DatabaseSystem {
    pub(crate) fn from_otel(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mysql" => Some(Self::MySql),
            "postgresql" => Some(Self::PostgreSql),
            "redis" => Some(Self::Redis),
            "valkey" => Some(Self::Valkey),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MySql => "MySQL",
            Self::PostgreSql => "PostgreSQL",
            Self::Redis => "Redis",
            Self::Valkey => "Valkey",
        }
    }
}

impl MetricSample {
    pub(crate) fn is_database(&self) -> bool {
        self.signal_subtype == MetricSignalSubtype::Database && self.database_system.is_some()
    }

    pub(crate) fn database_system_label(&self) -> &str {
        self.database_system
            .map(DatabaseSystem::label)
            .unwrap_or(crate::config::METRIC_EMPTY_FIELD)
    }
}
