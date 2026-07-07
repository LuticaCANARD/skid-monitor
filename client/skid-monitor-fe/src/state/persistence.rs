use super::DashboardState;
use crate::edge::PersistedEdgeState;

impl DashboardState {
    pub(in crate::state) fn persist_edge(&self, edge: PersistedEdgeState) {
        if let Some(storage) = &self.storage {
            storage.persist_edge(&edge);
        }
    }
}
