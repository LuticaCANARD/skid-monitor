use super::appearance::AppearanceMode;
use super::*;

#[test]
fn default_appearance_follows_system() {
    let settings = UiSettings::default();

    assert!(matches!(settings.appearance, AppearanceMode::System));
    assert!(settings.background_follows_appearance);
}
