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
