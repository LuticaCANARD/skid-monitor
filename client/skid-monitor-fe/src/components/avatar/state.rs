use crate::config;
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AvatarAlertState {
    Idle,
    Concerned,
    Urgent,
}

impl AvatarAlertState {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Concerned => "Concerned",
            Self::Urgent => "Urgent",
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
