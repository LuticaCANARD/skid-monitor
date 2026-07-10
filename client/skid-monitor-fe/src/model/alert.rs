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
