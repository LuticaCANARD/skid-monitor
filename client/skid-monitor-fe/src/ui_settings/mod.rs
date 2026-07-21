mod appearance;
mod background;
mod character_window;
mod window;

#[cfg(test)]
mod tests;

use crate::model::AvatarReactionProfile;
use appearance::{AppearanceMode, configure_theme_visuals, default_background_for_theme};
use background::{
    BackgroundImage, BackgroundTheme, dropped_image_path, load_background_texture,
    paint_cover_image,
};
use eframe::egui;

pub(crate) struct UiSettings {
    appearance: AppearanceMode,
    background: BackgroundTheme,
    background_follows_appearance: bool,
    background_image_path: String,
    background_image: Option<BackgroundImage>,
    background_image_error: Option<String>,
    avatar_applied: AvatarReactionProfile,
    avatar_applied_revision: u64,
    avatar_draft: AvatarReactionProfile,
    avatar_profile_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct SettingsChanges {
    pub(crate) alerts_enabled: Option<bool>,
    pub(crate) avatar_profile: Option<AvatarReactionProfile>,
    pub(crate) preview_character: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            background: BackgroundTheme::Obsidian,
            background_follows_appearance: true,
            background_image_path: String::new(),
            background_image: None,
            background_image_error: None,
            avatar_applied: AvatarReactionProfile::default(),
            avatar_applied_revision: 0,
            avatar_draft: AvatarReactionProfile::default(),
            avatar_profile_error: None,
        }
    }
}

impl UiSettings {
    pub(crate) fn new(
        avatar_profile: &AvatarReactionProfile,
        avatar_profile_revision: u64,
    ) -> Self {
        Self {
            avatar_applied: avatar_profile.clone(),
            avatar_applied_revision: avatar_profile_revision,
            avatar_draft: avatar_profile.clone(),
            ..Self::default()
        }
    }

    pub(crate) fn sync_avatar_profile(
        &mut self,
        avatar_profile: &AvatarReactionProfile,
        avatar_profile_revision: u64,
    ) {
        if self.avatar_applied_revision == avatar_profile_revision {
            return;
        }
        self.avatar_applied = avatar_profile.clone();
        self.avatar_applied_revision = avatar_profile_revision;
        self.avatar_draft = avatar_profile.clone();
        self.avatar_profile_error = None;
    }

    pub(crate) fn reject_avatar_profile(&mut self, error: impl Into<String>) {
        self.avatar_profile_error = Some(error.into());
    }

    pub(crate) fn apply_visuals(&self, ctx: &egui::Context) {
        configure_theme_visuals(ctx);
        ctx.set_theme(self.appearance.theme_preference());
    }

    pub(crate) fn paint_background(&mut self, ui: &egui::Ui) {
        self.sync_system_background(ui.ctx());
        ui.painter().rect_filled(
            ui.max_rect(),
            egui::CornerRadius::ZERO,
            self.background.fill(),
        );
        if let Some(image) = &self.background_image {
            paint_cover_image(ui, image);
        }
    }

    pub(crate) fn load_dropped_assets(&mut self, ctx: &egui::Context) {
        let mut loaded_character_asset = false;
        if let Some(path) = dropped_asset_path(ctx, "vrm") {
            self.avatar_draft.model_path = path;
            self.avatar_profile_error = None;
            loaded_character_asset = true;
        }
        let dropped_animations = dropped_asset_paths(ctx, "vrma");
        if !dropped_animations.is_empty() {
            loaded_character_asset = true;
            for path in dropped_animations {
                if self.avatar_draft.animation_paths.contains(&path) {
                    continue;
                }
                if self.avatar_draft.animation_paths.len()
                    >= crate::model::MAX_AVATAR_ANIMATION_PATHS
                {
                    self.avatar_profile_error = Some(format!(
                        "at most {} VRMA files are supported",
                        crate::model::MAX_AVATAR_ANIMATION_PATHS
                    ));
                    break;
                }
                self.avatar_draft.animation_paths.push(path);
                self.avatar_profile_error = None;
            }
        }
        if let Some(path) = dropped_asset_path(ctx, "wgsl") {
            self.avatar_draft.shader_path = path;
            self.avatar_profile_error = None;
            loaded_character_asset = true;
        }
        if loaded_character_asset {
            return;
        }
        if let Some(path) = dropped_image_path(ctx) {
            self.background_image_path = path;
            self.load_background_image(ctx);
        }
    }

    fn sync_background_for_appearance(&mut self, ctx: &egui::Context) {
        if self.background_follows_appearance {
            self.background = default_background_for_theme(self.effective_theme(ctx));
        }
    }

    fn sync_system_background(&mut self, ctx: &egui::Context) {
        if self.background_follows_appearance && self.appearance == AppearanceMode::System {
            self.background = default_background_for_theme(self.effective_theme(ctx));
        }
    }

    fn effective_theme(&self, ctx: &egui::Context) -> egui::Theme {
        match self.appearance {
            AppearanceMode::System => ctx.system_theme().unwrap_or(egui::Theme::Dark),
            AppearanceMode::Dark => egui::Theme::Dark,
            AppearanceMode::Light => egui::Theme::Light,
        }
    }

    fn load_background_image(&mut self, ctx: &egui::Context) {
        let path = self.background_image_path.trim();
        if path.is_empty() {
            self.background_image_error = Some("image path is required".to_string());
            return;
        }

        match load_background_texture(ctx, path) {
            Ok(image) => {
                self.background_image = Some(image);
                self.background_image_error = None;
            }
            Err(error) => {
                self.background_image_error = Some(error);
            }
        }
    }
}

fn dropped_asset_path(ctx: &egui::Context, expected_extension: &str) -> Option<String> {
    dropped_asset_paths(ctx, expected_extension)
        .into_iter()
        .next()
}

fn dropped_asset_paths(ctx: &egui::Context, expected_extension: &str) -> Vec<String> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .filter_map(|file| file.path.as_ref())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
            })
            .map(|path| path.display().to_string())
            .collect()
    })
}
