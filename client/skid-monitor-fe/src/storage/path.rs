use crate::config;
use std::path::PathBuf;

pub(super) fn state_db_path() -> PathBuf {
    if let Ok(path) = std::env::var(config::STATE_DB_PATH_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        let trimmed = state_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join("skid-monitor")
                .join(config::STATE_DB_DEFAULT_FILE);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join(".local")
                .join("state")
                .join("skid-monitor")
                .join(config::STATE_DB_DEFAULT_FILE);
        }
    }

    PathBuf::from(config::STATE_DB_DEFAULT_FILE)
}
