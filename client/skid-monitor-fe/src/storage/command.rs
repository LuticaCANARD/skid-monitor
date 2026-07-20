use super::AlertRecord;
use crate::edge::PersistedEdgeState;
use crate::model::AvatarReactionProfile;
use std::sync::mpsc::Sender;

pub(super) enum StorageCommand {
    UpsertEdge(PersistedEdgeState),
    DeleteEdge(String),
    RecordAlert(AlertRecord),
}

pub(super) enum StorageControlCommand {
    SaveAvatarProfile {
        profile: AvatarReactionProfile,
        result_tx: Sender<Result<(), String>>,
    },
    Shutdown {
        done_tx: Sender<()>,
    },
}
