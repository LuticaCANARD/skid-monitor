use super::DashboardState;
#[cfg(not(target_arch = "wasm32"))]
use super::PendingAvatarProfileSave;
use crate::alert::AlertStore;
use crate::edge::EdgeSignalDecorations;
use crate::model::{
    AlertSeverity, AlertSummary, AvatarReactionProfile, EventRow, MetricSample, NodeSummary,
    OperationalSummary, SignalCounters, Status,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::TryRecvError;

impl DashboardState {
    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    pub(crate) fn counters(&self) -> &SignalCounters {
        &self.counters
    }

    pub(crate) fn events(&self) -> &VecDeque<EventRow> {
        &self.events
    }

    pub(crate) fn metrics(&self) -> &VecDeque<MetricSample> {
        &self.metrics
    }

    pub(crate) fn metric_history(&self) -> &BTreeMap<String, VecDeque<f64>> {
        &self.metric_history
    }

    pub(crate) fn nodes(&self) -> &BTreeMap<String, NodeSummary> {
        &self.nodes
    }

    pub(crate) fn listeners(&self) -> &BTreeSet<String> {
        &self.listeners
    }

    pub(crate) fn edge_decorations(&self) -> &EdgeSignalDecorations {
        &self.edge_decorations
    }

    pub(crate) fn alerts(&self) -> &AlertStore {
        &self.alerts
    }

    pub(crate) fn alerts_enabled(&self) -> bool {
        self.alerts_enabled
    }

    pub(crate) fn avatar_profile(&self) -> &AvatarReactionProfile {
        &self.avatar_profile
    }

    pub(crate) fn avatar_profile_revision(&self) -> u64 {
        self.avatar_profile_revision
    }

    pub(crate) fn avatar_model_revision(&self) -> u64 {
        self.avatar_model_revision
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn avatar_profile_save_pending(&self) -> bool {
        self.pending_avatar_profile.is_some()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn avatar_profile_save_pending(&self) -> bool {
        false
    }

    pub(crate) fn set_avatar_profile(
        &mut self,
        profile: AvatarReactionProfile,
    ) -> Result<(), String> {
        let profile = profile.normalized()?;
        let storage = self.storage.as_ref().ok_or_else(|| {
            "state storage is unavailable; character profile was not applied".to_string()
        })?;

        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.pending_avatar_profile.is_some() {
                return Err("a character profile save is already in progress".to_string());
            }
            let result_rx = storage.queue_avatar_profile_save(&profile)?;
            self.pending_avatar_profile = Some(PendingAvatarProfileSave { profile, result_rx });
            Ok(())
        }

        #[cfg(target_arch = "wasm32")]
        {
            storage.persist_avatar_profile(&profile)?;
            self.commit_avatar_profile(profile);
            Ok(())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn poll_avatar_profile_save(&mut self) -> Option<Result<(), String>> {
        let result = match self.pending_avatar_profile.as_ref()?.result_rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err("character profile save result channel disconnected".to_string())
            }
        };
        let pending = self.pending_avatar_profile.take()?;

        match result {
            Ok(()) => {
                self.commit_avatar_profile(pending.profile);
                Some(Ok(()))
            }
            Err(error) => {
                self.push_event(
                    "settings",
                    format!("character profile save failed: {error}"),
                );
                Some(Err(error))
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn poll_avatar_profile_save(&mut self) -> Option<Result<(), String>> {
        None
    }

    fn commit_avatar_profile(&mut self, profile: AvatarReactionProfile) {
        let reload_model = self.avatar_profile.model_path != profile.model_path
            || self.avatar_profile.animation_paths != profile.animation_paths
            || self.avatar_profile == profile;
        if self.avatar_profile == profile {
            self.push_event("settings", "character reaction profile saved");
        } else {
            self.push_event(
                "settings",
                format!(
                    "character reaction profile changed to {}",
                    profile.model_name
                ),
            );
        }
        self.avatar_profile = profile;
        self.avatar_profile_revision = self.avatar_profile_revision.wrapping_add(1);
        if reload_model {
            self.avatar_model_revision = self.avatar_model_revision.wrapping_add(1);
        }
    }

    pub(crate) fn push_settings_error(&mut self, message: impl Into<String>) {
        self.push_event("settings", message);
    }

    pub(crate) fn set_alerts_enabled(&mut self, enabled: bool) {
        if self.alerts_enabled == enabled {
            return;
        }

        self.alerts_enabled = enabled;
        if enabled {
            self.push_event("settings", "alerts enabled");
        } else {
            self.alerts.clear();
            let cleared_edges = self.edge_decorations.clear_severities();
            for edge in cleared_edges {
                self.persist_edge(edge);
            }
            self.push_event("settings", "alerts disabled");
        }
    }

    pub(crate) fn alert_summary(&self) -> AlertSummary {
        let mut summary = self.alerts.summary();
        summary.enabled = self.alerts_enabled;
        if !self.alerts_enabled {
            summary.active_count = 0;
            summary.highest_severity = None;
        }
        summary
    }

    pub(crate) fn operational_summary(&self) -> OperationalSummary {
        let mut summary = OperationalSummary {
            agents: self.nodes.len(),
            listeners: self.listeners.len(),
            storage_enabled: self.storage.is_some(),
            ..OperationalSummary::default()
        };

        for node in self.nodes.values() {
            match self
                .edge_decorations
                .get(&node.endpoint, &node.node)
                .and_then(|edge| edge.severity)
            {
                Some(AlertSeverity::Critical) => summary.critical += 1,
                Some(AlertSeverity::Warning) => summary.warning += 1,
                None if node_signal_count(node) == 0 => summary.pending += 1,
                None => summary.online += 1,
            }
        }

        summary
    }
}

fn node_signal_count(node: &NodeSummary) -> usize {
    node.metric_points + node.spans + node.log_records
}
