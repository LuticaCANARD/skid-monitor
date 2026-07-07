use eframe::egui;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum AppearanceMode {
    System,
    Dark,
    Light,
}

impl AppearanceMode {
    pub(super) const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    pub(super) fn theme_preference(self) -> egui::ThemePreference {
        match self {
            Self::System => egui::ThemePreference::System,
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
        }
    }
}

pub(super) fn configure_theme_visuals(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Dark, egui::Visuals::dark());
    ctx.set_visuals_of(egui::Theme::Light, light_visuals());
}

pub(super) fn default_background_for_theme(
    theme: egui::Theme,
) -> super::background::BackgroundTheme {
    match theme {
        egui::Theme::Dark => super::background::BackgroundTheme::Obsidian,
        egui::Theme::Light => super::background::BackgroundTheme::Paper,
    }
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.window_fill = egui::Color32::from_rgb(248, 250, 253);
    visuals.panel_fill = egui::Color32::from_rgb(244, 247, 251);
    visuals.extreme_bg_color = egui::Color32::from_rgb(236, 241, 247);
    visuals
}
