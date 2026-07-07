use super::DashboardState;
use crate::config;
use crate::model::{AlertChange, AlertSeverity, AlertStatus, AlertTransition, EventRow};
use crate::utils::{format_event_time, push_capped};
use std::time::SystemTime;

impl DashboardState {
    pub(in crate::state) fn push_event(
        &mut self,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) {
        push_capped(
            &mut self.events,
            EventRow {
                time: format_event_time(SystemTime::now()),
                kind: kind.into(),
                message: message.into(),
            },
            config::MAX_EVENTS,
        );
    }

    pub(in crate::state) fn push_alert_change(&mut self, change: Option<AlertChange>) {
        let Some(change) = change else {
            return;
        };
        if let Some(storage) = &self.storage {
            storage.persist_alert(&change);
        }

        let status = match change.snapshot.status {
            AlertStatus::Firing => "firing",
            AlertStatus::Resolved => "resolved",
        };
        let kind = match change.transition {
            AlertTransition::Fired => "alert",
            AlertTransition::Resolved => "resolved",
        };
        let severity = severity_label(change.snapshot.severity);

        self.push_event(
            kind,
            format!(
                "{status} {severity} {} [{}] from {}@{}: {}",
                change.snapshot.summary,
                change.snapshot.rule_id,
                change.snapshot.source,
                change.snapshot.endpoint,
                change.snapshot.detail
            ),
        );
    }
}

fn severity_label(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Warning => "warning",
        AlertSeverity::Critical => "critical",
    }
}
