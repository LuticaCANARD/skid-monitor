use super::AlertSeverity;
use serde::{Deserialize, Serialize};
use std::path::Path;

const PROFILE_SCHEMA_VERSION: u32 = 1;
const MAX_MODEL_NAME_CHARS: usize = 64;
const MAX_MODEL_PATH_CHARS: usize = 4096;
const MAX_MESSAGE_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvatarMotion {
    Still,
    Pulse,
    Bounce,
    Shake,
}

impl AvatarMotion {
    pub(crate) const ALL: [Self; 4] = [Self::Still, Self::Pulse, Self::Bounce, Self::Shake];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Still => "Still",
            Self::Pulse => "Pulse",
            Self::Bounce => "Bounce",
            Self::Shake => "Shake",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct AvatarAction {
    pub(crate) motion: AvatarMotion,
    pub(crate) message: String,
}

impl Default for AvatarAction {
    fn default() -> Self {
        Self {
            motion: AvatarMotion::Still,
            message: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct AvatarReactionProfile {
    pub(crate) schema_version: u32,
    pub(crate) model_name: String,
    /// Native PNG/JPEG sprite or VRM avatar path. An empty path selects the built-in model.
    pub(crate) model_path: String,
    pub(crate) idle: AvatarAction,
    pub(crate) warning: AvatarAction,
    pub(crate) critical: AvatarAction,
}

impl Default for AvatarReactionProfile {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            model_name: "Skid".to_string(),
            model_path: String::new(),
            idle: AvatarAction {
                motion: AvatarMotion::Still,
                message: "All systems look calm.".to_string(),
            },
            warning: AvatarAction {
                motion: AvatarMotion::Pulse,
                message: "A warning needs attention.".to_string(),
            },
            critical: AvatarAction {
                motion: AvatarMotion::Shake,
                message: "Critical condition detected!".to_string(),
            },
        }
    }
}

impl AvatarReactionProfile {
    pub(crate) fn action_for(&self, severity: Option<AlertSeverity>) -> &AvatarAction {
        match severity {
            Some(AlertSeverity::Critical) => &self.critical,
            Some(AlertSeverity::Warning) => &self.warning,
            None => &self.idle,
        }
    }

    pub(crate) fn normalized(mut self) -> Result<Self, String> {
        if self.schema_version > PROFILE_SCHEMA_VERSION {
            return Err(format!(
                "character profile schema version {} is newer than supported version {PROFILE_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        self.schema_version = PROFILE_SCHEMA_VERSION;
        self.model_name = self.model_name.trim().to_string();
        self.model_path = self.model_path.trim().to_string();
        self.idle.message = self.idle.message.trim().to_string();
        self.warning.message = self.warning.message.trim().to_string();
        self.critical.message = self.critical.message.trim().to_string();
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(format!(
                "character profile schema version {} is not supported",
                self.schema_version
            ));
        }
        if self.model_name.is_empty() {
            return Err("character name is required".to_string());
        }
        if self.model_name.chars().count() > MAX_MODEL_NAME_CHARS {
            return Err(format!(
                "character name must be at most {MAX_MODEL_NAME_CHARS} characters"
            ));
        }
        if self.model_path.chars().count() > MAX_MODEL_PATH_CHARS {
            return Err(format!(
                "model path must be at most {MAX_MODEL_PATH_CHARS} characters"
            ));
        }
        if !self.model_path.is_empty() && !has_supported_model_extension(&self.model_path) {
            return Err(
                "character model must use a .png, .jpg, .jpeg, or .vrm extension".to_string(),
            );
        }

        for (state, action) in [
            ("idle", &self.idle),
            ("warning", &self.warning),
            ("critical", &self.critical),
        ] {
            if action.message.chars().count() > MAX_MESSAGE_CHARS {
                return Err(format!(
                    "{state} message must be at most {MAX_MESSAGE_CHARS} characters"
                ));
            }
        }

        Ok(())
    }
}

fn has_supported_model_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "vrm"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_maps_each_severity_to_a_safe_motion() {
        let profile = AvatarReactionProfile::default();

        assert_eq!(profile.action_for(None).motion, AvatarMotion::Still);
        assert_eq!(
            profile.action_for(Some(AlertSeverity::Warning)).motion,
            AvatarMotion::Pulse
        );
        assert_eq!(
            profile.action_for(Some(AlertSeverity::Critical)).motion,
            AvatarMotion::Shake
        );
    }

    #[test]
    fn profile_round_trips_as_versioned_json() {
        let profile = AvatarReactionProfile {
            model_name: "Operator Cat".to_string(),
            model_path: "/tmp/operator-cat.png".to_string(),
            ..AvatarReactionProfile::default()
        };

        let json = serde_json::to_string(&profile).expect("serialize profile");
        let restored: AvatarReactionProfile =
            serde_json::from_str(&json).expect("deserialize profile");

        assert_eq!(restored, profile);
        assert_eq!(restored.schema_version, PROFILE_SCHEMA_VERSION);
    }

    #[test]
    fn native_vrm_model_extension_is_accepted() {
        let profile = AvatarReactionProfile {
            model_path: "/tmp/operator-cat.vrm".to_string(),
            ..AvatarReactionProfile::default()
        };

        assert!(profile.validate().is_ok());
    }

    #[test]
    fn invalid_model_extension_is_rejected() {
        let profile = AvatarReactionProfile {
            model_path: "/tmp/operator-cat.glb".to_string(),
            ..AvatarReactionProfile::default()
        };

        assert!(profile.validate().is_err());
    }

    #[test]
    fn future_profile_schema_is_rejected_instead_of_silently_downgraded() {
        let profile = AvatarReactionProfile {
            schema_version: PROFILE_SCHEMA_VERSION + 1,
            ..AvatarReactionProfile::default()
        };

        assert!(profile.normalized().is_err());
    }
}
