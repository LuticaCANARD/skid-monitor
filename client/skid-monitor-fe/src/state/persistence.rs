use super::DashboardState;
use crate::edge::PersistedEdgeState;

impl DashboardState {
    pub(in crate::state) fn persist_edge(&self, edge: PersistedEdgeState) {
        if let Some(storage) = &self.storage {
            storage.persist_edge(&edge);
        }
    }

    pub(in crate::state) fn forget_edge(&self, key: &str) {
        if let Some(storage) = &self.storage {
            storage.delete_edge(key);
        }
    }
}
