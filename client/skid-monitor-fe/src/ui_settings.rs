use crate::config;
use eframe::egui::{self, RichText};
use std::path::Path;

pub(crate) struct UiSettings {
    pub(crate) appearance: AppearanceMode,
    pub(crate) background: BackgroundTheme,
    background_follows_appearance: bool,
    background_image_path: String,
    background_image: Option<BackgroundImage>,
    background_image_error: Option<String>,
}

pub(crate) struct SettingsChanges {
    pub(crate) alerts_enabled: Option<bool>,
}

struct BackgroundImage {
    texture: egui::TextureHandle,
    size: egui::Vec2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum AppearanceMode {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum BackgroundTheme {
    Obsidian,
    Graphite,
    DeepGreen,
    Paper,
    Porcelain,
}

const BACKGROUND_THEMES: [BackgroundTheme; 5] = [
    BackgroundTheme::Obsidian,
    BackgroundTheme::Graphite,
    BackgroundTheme::DeepGreen,
    BackgroundTheme::Paper,
    BackgroundTheme::Porcelain,
];

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

    pub(crate) fn show_window(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        alerts_enabled: bool,
    ) -> SettingsChanges {
        let mut changes = SettingsChanges {
            alerts_enabled: None,
        };
        let mut window_open = *open;

        egui::Window::new("Settings")
            .id(egui::Id::new("settings-window"))
            .open(&mut window_open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                self.sync_system_background(ctx);
                self.show_appearance(ui, ctx);
                ui.add_space(config::SECTION_GAP);
                self.show_background(ui, ctx);
                ui.add_space(config::SECTION_GAP);
                show_alert_settings(ui, alerts_enabled, &mut changes);
            });

        *open = window_open;
        changes
    }

    fn show_appearance(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("Appearance").strong());
        ui.horizontal(|ui| {
            for mode in AppearanceMode::ALL {
                if ui
                    .selectable_value(&mut self.appearance, mode, mode.label())
                    .clicked()
                {
                    self.apply_visuals(ctx);
                    self.sync_background_for_appearance(ctx);
                }
            }
        });
    }

    fn show_background(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("Background").strong());
        for theme in BACKGROUND_THEMES {
            if background_theme_row(ui, self.background == theme, theme).clicked() {
                self.background = theme;
                self.background_follows_appearance = false;
            }
        }

        ui.add_space(config::SECTION_GAP);
        ui.label(RichText::new("Image").strong());
        ui.label(RichText::new("enter a path or drop a png/jpeg file").small());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.background_image_path)
                    .desired_width(320.0)
                    .hint_text("/path/to/background.jpg"),
            );
            if ui.button("Load").clicked() {
                self.load_background_image(ctx);
            }
            if ui.button("Clear").clicked() {
                self.background_image = None;
                self.background_image_error = None;
            }
        });
        if let Some(error) = &self.background_image_error {
            ui.label(RichText::new(error).color(config::STATUS_ERROR_COLOR));
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

impl AppearanceMode {
    const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    fn theme_preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
        }
    }
}

impl BackgroundTheme {
    fn label(self) -> &'static str {
        match self {
            Self::Obsidian => "Obsidian",
            Self::Graphite => "Graphite",
            Self::DeepGreen => "Deep green",
            Self::Paper => "Paper",
            Self::Porcelain => "Porcelain",
        }
    }

    fn fill(self) -> egui::Color32 {
        match self {
            Self::Obsidian => config::PAGE_BACKGROUND,
            Self::Graphite => egui::Color32::from_rgb(22, 23, 25),
            Self::DeepGreen => egui::Color32::from_rgb(12, 24, 21),
            Self::Paper => egui::Color32::from_rgb(241, 244, 248),
            Self::Porcelain => egui::Color32::from_rgb(250, 251, 253),
        }
    }
}

fn configure_theme_visuals(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::dark());
    ctx.set_visuals_of(egui::Theme::Light, light_visuals());
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.window_fill = egui::Color32::from_rgb(248, 250, 253);
    visuals.panel_fill = egui::Color32::from_rgb(244, 247, 251);
    visuals.extreme_bg_color = egui::Color32::from_rgb(236, 241, 247);
    visuals
}

fn default_background_for_theme(theme: egui::Theme) -> BackgroundTheme {
    match theme {
        egui::Theme::Dark => BackgroundTheme::Obsidian,
        egui::Theme::Light => BackgroundTheme::Paper,
    }
}

fn show_alert_settings(ui: &mut egui::Ui, alerts_enabled: bool, changes: &mut SettingsChanges) {
    ui.label(RichText::new("Alerts").strong());
    let mut next = alerts_enabled;
    if ui.checkbox(&mut next, "Enable alerts").changed() {
        changes.alerts_enabled = Some(next);
    }
}

fn background_theme_row(
    ui: &mut egui::Ui,
    selected: bool,
    theme: BackgroundTheme,
) -> egui::Response {
    ui.horizontal(|ui| {
        let desired = egui::vec2(24.0, 24.0);
        let (rect, swatch_response) = ui.allocate_exact_size(desired, egui::Sense::click());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), theme.fill());
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(
                if selected { 2.0 } else { 1.0 },
                if selected {
                    config::STATUS_LISTENING_COLOR
                } else {
                    config::STAT_TILE_BORDER
                },
            ),
            egui::StrokeKind::Inside,
        );
        let label_response = ui.selectable_label(selected, theme.label());
        swatch_response.union(label_response)
    })
    .inner
}

fn load_background_texture(
    ctx: &egui::Context,
    path: impl AsRef<Path>,
) -> Result<BackgroundImage, String> {
    let path = path.as_ref();
    let image = image::open(path)
        .map_err(|error| format!("failed to load {}: {error}", path.display()))?
        .to_rgba8();
    let width = usize::try_from(image.width()).map_err(|_| "image width is too large")?;
    let height = usize::try_from(image.height()).map_err(|_| "image height is too large")?;
    let color_image = egui::ColorImage::from_rgba_unmultiplied([width, height], image.as_raw());
    let texture = ctx.load_texture(
        format!("background-image:{}", path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    );

    Ok(BackgroundImage {
        texture,
        size: egui::vec2(width as f32, height as f32),
    })
}

fn paint_cover_image(ui: &egui::Ui, image: &BackgroundImage) {
    let rect = ui.max_rect();
    if image.size.x <= 0.0 || image.size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    let image_aspect = image.size.x / image.size.y;
    let rect_aspect = rect.width() / rect.height();
    let uv = if image_aspect > rect_aspect {
        let visible_width = (rect_aspect / image_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible_width) * 0.5;
        egui::Rect::from_min_max(egui::pos2(inset, 0.0), egui::pos2(1.0 - inset, 1.0))
    } else {
        let visible_height = (image_aspect / rect_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible_height) * 0.5;
        egui::Rect::from_min_max(egui::pos2(0.0, inset), egui::pos2(1.0, 1.0 - inset))
    };

    ui.painter()
        .image(image.texture.id(), rect, uv, egui::Color32::WHITE);
}

fn dropped_image_path(ctx: &egui::Context) -> Option<String> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .find_map(|file| file.path.as_ref())
            .map(|path| path.display().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_appearance_follows_system() {
        let settings = UiSettings::default();

        assert!(matches!(settings.appearance, AppearanceMode::System));
        assert!(settings.background_follows_appearance);
    }
}
