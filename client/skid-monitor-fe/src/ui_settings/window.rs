use super::appearance::AppearanceMode;
use super::background::{BACKGROUND_THEMES, background_theme_row};
use super::{SettingsChanges, UiSettings};
use crate::config;
use crate::model::{AvatarAction, AvatarMotion};
use eframe::egui::{self, RichText};

impl UiSettings {
    pub(crate) fn show_window(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        alerts_enabled: bool,
        avatar_profile_save_pending: bool,
        avatar_model_path: Option<&str>,
        avatar_model_error: Option<&str>,
    ) -> SettingsChanges {
        let mut changes = SettingsChanges::default();
        let mut window_open = *open;

        egui::Window::new("Settings")
            .id(egui::Id::new("settings-window"))
            .open(&mut window_open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(680.0)
                    .show(ui, |ui| {
                        self.sync_system_background(ctx);
                        self.show_appearance(ui, ctx);
                        ui.add_space(config::SECTION_GAP);
                        self.show_background(ui, ctx);
                        ui.add_space(config::SECTION_GAP);
                        show_alert_settings(ui, alerts_enabled, &mut changes);
                        ui.add_space(config::SECTION_GAP);
                        ui.add_enabled_ui(!avatar_profile_save_pending, |ui| {
                            self.show_character_settings(
                                ui,
                                avatar_model_path,
                                avatar_model_error,
                                &mut changes,
                            );
                        });
                        if avatar_profile_save_pending {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Saving character profile…");
                            });
                        }
                    });
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

    fn show_character_settings(
        &mut self,
        ui: &mut egui::Ui,
        avatar_model_path: Option<&str>,
        avatar_model_error: Option<&str>,
        changes: &mut SettingsChanges,
    ) {
        ui.label(RichText::new("Character reactions").strong());
        ui.label(
            RichText::new(
                "Choose a 2D sprite or VRM avatar and map each server alert state to a safe visual action.",
            )
            .small(),
        );

        ui.add_space(config::SECTION_GAP);
        ui.label("Character name");
        ui.add(
            egui::TextEdit::singleline(&mut self.avatar_draft.model_name)
                .desired_width(320.0)
                .hint_text("Skid"),
        );

        ui.label("PNG/JPEG/VRM model path");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.avatar_draft.model_path)
                    .desired_width(360.0)
                    .hint_text("empty uses the built-in character; .vrm needs high-spec"),
            );
            if ui.button("Clear").clicked() {
                self.avatar_draft.model_path.clear();
            }
        });
        #[cfg(target_arch = "wasm32")]
        ui.label(
            RichText::new("Local PNG/JPEG/VRM paths currently load in the native client only.")
                .small(),
        );
        #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
        ui.label(
            RichText::new(
                "Drop a .vrm file anywhere or enter its path. VRM 0.x/1.0 loads as a static 3D avatar.",
            )
            .small(),
        );
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "high-spec")))]
        ui.label(
            RichText::new(
                "VRM rendering requires: cargo run -p skid-monitor-fe --no-default-features --features high-spec",
            )
            .small(),
        );

        if let Some(error) = avatar_model_error.filter(|_| {
            avatar_model_path == Some(self.avatar_draft.model_path.trim())
                && !self.avatar_draft.model_path.trim().is_empty()
        }) {
            ui.label(RichText::new(error).color(config::STATUS_ERROR_COLOR));
        }

        ui.add_space(config::SECTION_GAP);
        show_action_editor(ui, "idle", "Healthy", &mut self.avatar_draft.idle);
        show_action_editor(ui, "warning", "Warning", &mut self.avatar_draft.warning);
        show_action_editor(ui, "critical", "Critical", &mut self.avatar_draft.critical);

        if let Some(error) = &self.avatar_profile_error {
            ui.label(RichText::new(error).color(config::STATUS_ERROR_COLOR));
        }

        ui.horizontal(|ui| {
            if ui.button("Apply character profile").clicked() {
                match self.avatar_draft.clone().normalized() {
                    Ok(profile) => {
                        self.avatar_draft = profile.clone();
                        self.avatar_profile_error = None;
                        changes.avatar_profile = Some(profile);
                    }
                    Err(error) => self.avatar_profile_error = Some(error),
                }
            }
            if ui.button("Reset draft").clicked() {
                self.avatar_draft = Default::default();
                self.avatar_profile_error = None;
            }
        });
        ui.label(RichText::new("Apply to save draft changes.").small());
    }
}

fn show_alert_settings(ui: &mut egui::Ui, alerts_enabled: bool, changes: &mut SettingsChanges) {
    ui.label(RichText::new("Alerts").strong());
    let mut next = alerts_enabled;
    if ui.checkbox(&mut next, "Enable alerts").changed() {
        changes.alerts_enabled = Some(next);
    }
}

fn show_action_editor(ui: &mut egui::Ui, id: &str, label: &str, action: &mut AvatarAction) {
    ui.group(|ui| {
        ui.label(RichText::new(label).strong());
        ui.horizontal(|ui| {
            ui.label("Motion");
            egui::ComboBox::from_id_salt(("avatar-motion", id))
                .selected_text(action.motion.label())
                .show_ui(ui, |ui| {
                    for motion in AvatarMotion::ALL {
                        ui.selectable_value(&mut action.motion, motion, motion.label());
                    }
                });
        });
        ui.label("Message");
        ui.add(
            egui::TextEdit::singleline(&mut action.message)
                .desired_width(440.0)
                .hint_text("optional speech bubble text"),
        );
    });
}
