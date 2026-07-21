use super::AlertSeverity;
use serde::{Deserialize, Serialize};
use std::path::Path;

const PROFILE_SCHEMA_VERSION: u32 = 3;
const MAX_MODEL_NAME_CHARS: usize = 64;
const MAX_MODEL_PATH_CHARS: usize = 4096;
const MAX_MESSAGE_CHARS: usize = 160;
const MAX_EXPRESSION_NAME_CHARS: usize = 64;
pub(crate) const MAX_AVATAR_ANIMATION_PATHS: usize = 8;

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
    /// VRM preset or custom expression name. Missing expressions are a safe no-op.
    pub(crate) expression: String,
}

impl Default for AvatarAction {
    fn default() -> Self {
        Self {
            motion: AvatarMotion::Still,
            message: String::new(),
            expression: String::new(),
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
    /// Legacy v2 VRMA path. Normalization migrates it into `animation_paths`.
    #[serde(skip_serializing)]
    pub(crate) animation_path: String,
    /// Optional VRM Animation (`.vrma`) layers. Every clip is available to the mixer.
    pub(crate) animation_paths: Vec<String>,
    pub(crate) animation_crossfade_seconds: f32,
    pub(crate) spring_bone_enabled: bool,
    pub(crate) look_at_enabled: bool,
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
            animation_path: String::new(),
            animation_paths: Vec::new(),
            animation_crossfade_seconds: 0.25,
            spring_bone_enabled: true,
            look_at_enabled: true,
            idle: AvatarAction {
                motion: AvatarMotion::Still,
                message: "All systems look calm.".to_string(),
                expression: "neutral".to_string(),
            },
            warning: AvatarAction {
                motion: AvatarMotion::Pulse,
                message: "A warning needs attention.".to_string(),
                expression: "surprised".to_string(),
            },
            critical: AvatarAction {
                motion: AvatarMotion::Shake,
                message: "Critical condition detected!".to_string(),
                expression: "angry".to_string(),
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
        self.animation_path = self.animation_path.trim().to_string();
        let mut animation_paths = Vec::new();
        for path in self.animation_paths {
            let path = path.trim().to_string();
            if !path.is_empty() && !animation_paths.contains(&path) {
                animation_paths.push(path);
            }
        }
        self.animation_paths = animation_paths;
        if self.animation_paths.is_empty() && !self.animation_path.is_empty() {
            self.animation_paths.push(self.animation_path.clone());
        }
        self.animation_path.clear();
        self.idle.message = self.idle.message.trim().to_string();
        self.warning.message = self.warning.message.trim().to_string();
        self.critical.message = self.critical.message.trim().to_string();
        self.idle.expression = self.idle.expression.trim().to_string();
        self.warning.expression = self.warning.expression.trim().to_string();
        self.critical.expression = self.critical.expression.trim().to_string();
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
        if !self.animation_path.is_empty() {
            return Err("legacy animation_path must be normalized before saving".to_string());
        }
        if self.animation_paths.len() > MAX_AVATAR_ANIMATION_PATHS {
            return Err(format!(
                "at most {MAX_AVATAR_ANIMATION_PATHS} VRMA animation files are supported"
            ));
        }
        if !self.animation_paths.is_empty() && !has_extension(&self.model_path, "vrm") {
            return Err("VRMA animations require a .vrm character model".to_string());
        }
        for path in &self.animation_paths {
            if path.chars().count() > MAX_MODEL_PATH_CHARS {
                return Err(format!(
                    "animation path must be at most {MAX_MODEL_PATH_CHARS} characters"
                ));
            }
            if !has_extension(path, "vrma") {
                return Err("character animations must use a .vrma extension".to_string());
            }
        }
        if !self.animation_crossfade_seconds.is_finite()
            || !(0.0..=2.0).contains(&self.animation_crossfade_seconds)
        {
            return Err("animation crossfade must be between 0 and 2 seconds".to_string());
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
            if action.expression.chars().count() > MAX_EXPRESSION_NAME_CHARS {
                return Err(format!(
                    "{state} expression must be at most {MAX_EXPRESSION_NAME_CHARS} characters"
                ));
            }
            if action.expression.chars().any(char::is_control) {
                return Err(format!(
                    "{state} expression must not contain control characters"
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

fn has_extension(path: &str, expected: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
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

        assert!(!json.contains("\"animation_path\""));
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
    fn vrma_animation_is_accepted_with_a_vrm_model() {
        let profile = AvatarReactionProfile {
            model_path: "/tmp/operator-cat.vrm".to_string(),
            animation_paths: vec![
                "/tmp/operator-cat-idle.vrma".to_string(),
                "/tmp/operator-cat-dance.vrma".to_string(),
            ],
            ..AvatarReactionProfile::default()
        };

        assert!(profile.validate().is_ok());
    }

    #[test]
    fn vrma_animation_without_a_vrm_model_is_rejected() {
        let profile = AvatarReactionProfile {
            model_path: "/tmp/operator-cat.png".to_string(),
            animation_paths: vec!["/tmp/operator-cat.vrma".to_string()],
            ..AvatarReactionProfile::default()
        };

        assert!(profile.validate().is_err());
    }

    #[test]
    fn animation_paths_are_trimmed_and_deduplicated() {
        let profile = AvatarReactionProfile {
            model_path: "/tmp/operator-cat.vrm".to_string(),
            animation_paths: vec![
                " /tmp/idle.vrma ".to_string(),
                "/tmp/idle.vrma".to_string(),
                String::new(),
            ],
            ..AvatarReactionProfile::default()
        };

        let normalized = profile.normalized().expect("normalize animation paths");

        assert_eq!(normalized.animation_paths, ["/tmp/idle.vrma"]);
    }

    #[test]
    fn version_one_profile_migrates_with_no_animation_paths() {
        let json = r#"{
            "schema_version": 1,
            "model_name": "Old Skid",
            "model_path": "/tmp/old.vrm",
            "idle": {"motion":"still","message":"ok"},
            "warning": {"motion":"pulse","message":"warn"},
            "critical": {"motion":"shake","message":"critical"}
        }"#;
        let profile: AvatarReactionProfile = serde_json::from_str(json).expect("old profile");
        let migrated = profile.normalized().expect("migrate old profile");

        assert_eq!(migrated.schema_version, PROFILE_SCHEMA_VERSION);
        assert!(migrated.animation_path.is_empty());
        assert!(migrated.animation_paths.is_empty());
    }

    #[test]
    fn version_two_profile_migrates_the_single_animation_path() {
        let mut profile = AvatarReactionProfile {
            schema_version: 2,
            model_path: "/tmp/old.vrm".to_string(),
            animation_path: "/tmp/old.vrma".to_string(),
            ..AvatarReactionProfile::default()
        };
        profile.animation_paths.clear();

        let migrated = profile.normalized().expect("migrate v2 profile");

        assert!(migrated.animation_path.is_empty());
        assert_eq!(migrated.animation_paths, ["/tmp/old.vrma"]);
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
