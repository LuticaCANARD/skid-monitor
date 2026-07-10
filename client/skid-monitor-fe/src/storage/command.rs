use super::AlertRecord;
use crate::edge::PersistedEdgeState;

pub(super) enum StorageCommand {
    UpsertEdge(PersistedEdgeState),
    DeleteEdge(String),
    RecordAlert(AlertRecord),
}
