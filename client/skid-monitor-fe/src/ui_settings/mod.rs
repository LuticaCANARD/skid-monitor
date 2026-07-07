mod appearance;
mod background;
mod window;

#[cfg(test)]
mod tests;

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
}

pub(crate) struct SettingsChanges {
    pub(crate) alerts_enabled: Option<bool>,
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
        }
    }
}

impl UiSettings {
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

    pub(crate) fn load_dropped_image(&mut self, ctx: &egui::Context) {
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
