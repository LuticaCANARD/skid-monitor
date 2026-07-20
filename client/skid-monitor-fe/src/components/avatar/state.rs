use crate::config;
use crate::model::AlertSeverity;
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AvatarAlertState {
    Idle,
    Concerned,
    Urgent,
}

impl AvatarAlertState {
    pub(super) fn from_severity(severity: Option<AlertSeverity>) -> Self {
        match severity {
            Some(AlertSeverity::Critical) => Self::Urgent,
            Some(AlertSeverity::Warning) => Self::Concerned,
            None => Self::Idle,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Idle => "Healthy",
            Self::Concerned => "Warning",
            Self::Urgent => "Critical",
        }
    }

    pub(super) fn accent(self) -> Color32 {
        match self {
            Self::Idle => config::ALERT_CLEAR_COLOR,
            Self::Concerned => config::ALERT_WARNING_COLOR,
            Self::Urgent => config::ALERT_CRITICAL_COLOR,
        }
    }
}
