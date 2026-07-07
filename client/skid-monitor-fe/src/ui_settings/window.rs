use super::appearance::AppearanceMode;
use super::background::{BACKGROUND_THEMES, background_theme_row};
use super::{SettingsChanges, UiSettings};
use crate::config;
use eframe::egui::{self, RichText};

impl UiSettings {
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
}

fn show_alert_settings(ui: &mut egui::Ui, alerts_enabled: bool, changes: &mut SettingsChanges) {
    ui.label(RichText::new("Alerts").strong());
    let mut next = alerts_enabled;
    if ui.checkbox(&mut next, "Enable alerts").changed() {
        changes.alerts_enabled = Some(next);
    }
}
