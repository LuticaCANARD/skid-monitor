use super::appearance::AppearanceMode;
use super::*;

#[test]
fn default_appearance_follows_system() {
    let settings = UiSettings::default();

    assert!(matches!(settings.appearance, AppearanceMode::System));
    assert!(settings.background_follows_appearance);
}

#[test]
fn editor_starts_with_the_persisted_character_profile() {
    let profile = AvatarReactionProfile {
        model_name: "Operator Cat".to_string(),
        model_path: "/tmp/operator-cat.png".to_string(),
        ..AvatarReactionProfile::default()
    };

    let settings = UiSettings::new(&profile, 7);

    assert_eq!(settings.avatar_applied, profile);
    assert_eq!(settings.avatar_applied_revision, 7);
    assert_eq!(settings.avatar_draft, profile);
}

#[test]
fn editor_switches_to_a_new_scoped_character_profile() {
    let first = AvatarReactionProfile {
        model_name: "Tenant One".to_string(),
        ..AvatarReactionProfile::default()
    };
    let second = AvatarReactionProfile {
        model_name: "Tenant Two".to_string(),
        ..AvatarReactionProfile::default()
    };
    let mut settings = UiSettings::new(&first, 1);
    settings.avatar_draft.model_name = "Unsaved draft".to_string();

    settings.sync_avatar_profile(&second, 2);

    assert_eq!(settings.avatar_applied, second);
    assert_eq!(settings.avatar_draft, second);
    assert!(settings.avatar_profile_error.is_none());
}

#[test]
fn editor_resets_draft_when_scope_revision_changes_with_the_same_profile() {
    let profile = AvatarReactionProfile::default();
    let mut settings = UiSettings::new(&profile, 1);
    settings.avatar_draft.model_name = "Unsaved tenant A draft".to_string();
    settings.avatar_profile_error = Some("tenant A error".to_string());

    settings.sync_avatar_profile(&profile, 2);

    assert_eq!(settings.avatar_applied_revision, 2);
    assert_eq!(settings.avatar_draft, profile);
    assert!(settings.avatar_profile_error.is_none());
}
