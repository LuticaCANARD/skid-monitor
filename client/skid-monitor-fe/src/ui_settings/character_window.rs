use super::{SettingsChanges, UiSettings};
use crate::config;
use crate::model::{AvatarAction, AvatarMotion};
use eframe::egui::{self, RichText};

impl UiSettings {
    pub(crate) fn show_character_window(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        avatar_profile_save_pending: bool,
        avatar_model_path: Option<&str>,
        avatar_model_error: Option<&str>,
    ) -> SettingsChanges {
        let mut changes = SettingsChanges::default();
        let mut window_open = *open;

        egui::Window::new("Character")
            .id(egui::Id::new("character-settings-window"))
            .open(&mut window_open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(680.0)
                    .show(ui, |ui| {
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
                "Drop a .vrm file anywhere or enter its path. VRM 0.x/1.0 loads in the native 3D viewport.",
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

        ui.label("Optional VRMA animation files");
        let mut remove_animation = None;
        for (index, path) in self.avatar_draft.animation_paths.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(path)
                        .desired_width(360.0)
                        .hint_text("path to a .vrma file"),
                );
                if ui.button("Remove").clicked() {
                    remove_animation = Some(index);
                }
            });
        }
        if let Some(index) = remove_animation {
            self.avatar_draft.animation_paths.remove(index);
        }
        if self.avatar_draft.animation_paths.len() < crate::model::MAX_AVATAR_ANIMATION_PATHS
            && ui.button("Add VRMA clip").clicked()
        {
            self.avatar_draft.animation_paths.push(String::new());
        }
        ui.horizontal(|ui| {
            ui.label("Clip crossfade");
            ui.add(
                egui::Slider::new(
                    &mut self.avatar_draft.animation_crossfade_seconds,
                    0.0..=2.0,
                )
                .suffix(" s"),
            );
            ui.checkbox(
                &mut self.avatar_draft.spring_bone_enabled,
                "SpringBone physics",
            );
            ui.checkbox(&mut self.avatar_draft.look_at_enabled, "Pointer look-at");
        });
        #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
        ui.label(
            RichText::new(
                "Drop one or more .vrma files. All clips are FK-retargeted and crossfaded by the runtime mixer; an empty list uses embedded glTF clips.",
            )
            .small(),
        );

        ui.add_space(config::SECTION_GAP);
        show_action_editor(ui, "idle", "Healthy", &mut self.avatar_draft.idle);
        show_action_editor(ui, "warning", "Warning", &mut self.avatar_draft.warning);
        show_action_editor(ui, "critical", "Critical", &mut self.avatar_draft.critical);

        if let Some(error) = &self.avatar_profile_error {
            ui.label(RichText::new(error).color(config::STATUS_ERROR_COLOR));
        }

        ui.horizontal(|ui| {
            if ui.button("Apply & preview character").clicked() {
                match self.avatar_draft.clone().normalized() {
                    Ok(profile) => {
                        self.avatar_draft = profile.clone();
                        self.avatar_profile_error = None;
                        changes.avatar_profile = Some(profile);
                        changes.preview_character = true;
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
        ui.label("VRM expression");
        ui.add(
            egui::TextEdit::singleline(&mut action.expression)
                .desired_width(240.0)
                .hint_text("preset or custom name, e.g. happy"),
        );
    });
}
