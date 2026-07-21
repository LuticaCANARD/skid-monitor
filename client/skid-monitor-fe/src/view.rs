use crate::components::{
    avatar::{self, AvatarModelCache},
    counters, header,
    header::HeaderAction,
    layout::{
        ContentLayout, LayoutMode, PanelLimits, centered_content, remaining_height, section_gap,
    },
};
use crate::config;
use crate::model::AvatarReactionProfile;
use crate::pages::{
    detail,
    overview::{self, OverviewAction, OverviewState},
};
use crate::state::DashboardState;
use crate::ui_settings::UiSettings;
use eframe::egui;
use std::time::Duration;

pub(crate) struct ControlRoomUiState {
    selected_node_key: Option<String>,
    overview: OverviewState,
    settings_open: bool,
    character_settings_open: bool,
    character_preview_open: bool,
    show_avatar: bool,
    settings: UiSettings,
    avatar_model: AvatarModelCache,
    avatar_model_revision: u64,
}

impl ControlRoomUiState {
    pub(crate) fn new(
        ctx: &egui::Context,
        avatar_profile: &AvatarReactionProfile,
        avatar_profile_revision: u64,
        avatar_model_revision: u64,
    ) -> Self {
        let settings = UiSettings::new(avatar_profile, avatar_profile_revision);
        settings.apply_visuals(ctx);
        let mut avatar_model = AvatarModelCache::default();
        avatar_model.sync(ctx, avatar_profile);
        let character_preview_open = !avatar_profile.model_path.trim().is_empty();

        Self {
            selected_node_key: None,
            overview: OverviewState::default(),
            settings_open: false,
            character_settings_open: false,
            character_preview_open,
            show_avatar: false,
            settings,
            avatar_model,
            avatar_model_revision,
        }
    }
}

pub(crate) struct ControlRoomView<'a> {
    state: &'a mut DashboardState,
    ui_state: &'a mut ControlRoomUiState,
}

impl<'a> ControlRoomView<'a> {
    pub(crate) fn new(state: &'a mut DashboardState, ui_state: &'a mut ControlRoomUiState) -> Self {
        Self { state, ui_state }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        if let Some(Err(error)) = self.state.poll_avatar_profile_save() {
            self.ui_state.settings.reject_avatar_profile(error);
        }
        let avatar_profile_revision = self.state.avatar_profile_revision();
        self.ui_state
            .settings
            .sync_avatar_profile(self.state.avatar_profile(), avatar_profile_revision);
        let avatar_model_revision = self.state.avatar_model_revision();
        if self.ui_state.avatar_model_revision != avatar_model_revision {
            self.ui_state.avatar_model.invalidate();
            self.ui_state.avatar_model_revision = avatar_model_revision;
        }
        self.ui_state
            .avatar_model
            .sync(ui.ctx(), self.state.avatar_profile());
        if self.state.avatar_profile_save_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        self.ui_state.settings.load_dropped_assets(ui.ctx());
        self.ui_state.settings.paint_background(ui);

        egui::Frame::default()
            .inner_margin(egui::Margin::same(config::CONTENT_FRAME_MARGIN))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("control-room-page-scroll")
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, _viewport| {
                        let content = ContentLayout::for_viewport(ui.clip_rect().size());

                        centered_content(ui, content, |ui| {
                            let panel_width = ui.available_width();
                            let layout = LayoutMode::for_width(panel_width);
                            let compact = layout.is_compact();

                            match header::show(
                                ui,
                                compact,
                                self.state.status(),
                                self.state.alert_summary(),
                                self.state.operational_summary(),
                            ) {
                                HeaderAction::OpenCharacter => {
                                    self.ui_state.character_settings_open = true;
                                }
                                HeaderAction::OpenSettings => {
                                    self.ui_state.settings_open = true;
                                }
                                HeaderAction::None => {}
                            }
                            ui.add_space(config::HEADER_COUNTER_GAP);
                            counters::show(ui, self.state.counters());
                            ui.add_space(config::SECTION_GAP);

                            let selected_key = self.current_selected_key();
                            if self.ui_state.selected_node_key != selected_key {
                                self.ui_state.selected_node_key = selected_key.clone();
                            }

                            let limits = PanelLimits::for_remaining_height(
                                remaining_height(ui, content),
                                layout,
                                section_gap(ui),
                            );
                            if let Some(key) = selected_key.as_deref() {
                                if matches!(
                                    detail::show(
                                        ui,
                                        compact,
                                        panel_width,
                                        layout,
                                        limits,
                                        self.state,
                                        key,
                                        &mut self.ui_state.show_avatar,
                                        &self.ui_state.avatar_model,
                                    ),
                                    Some(detail::DetailAction::BackToOverview)
                                ) {
                                    self.ui_state.selected_node_key = None;
                                }
                            } else {
                                let action = overview::show(
                                    ui,
                                    compact,
                                    panel_width,
                                    limits,
                                    self.state,
                                    &mut self.ui_state.overview,
                                );
                                if let Some(action) = action {
                                    self.handle_overview_action(action);
                                }
                            }
                            ui.add_space(content.bottom_margin);
                        });
                    });
            });

        if self.ui_state.settings_open {
            self.show_settings_window(ui.ctx());
        }
        if self.ui_state.character_settings_open {
            self.show_character_settings_window(ui.ctx());
        }
        if self.ui_state.character_preview_open {
            self.show_character_preview(ui.ctx());
        }
    }

    fn current_selected_key(&self) -> Option<String> {
        detail::selected_key(self.ui_state.selected_node_key.as_deref(), self.state)
    }

    fn handle_overview_action(&mut self, action: OverviewAction) {
        match action {
            OverviewAction::Select(key) => {
                self.ui_state.selected_node_key = Some(key);
                self.ui_state.overview.select_agent();
            }
            OverviewAction::StartAdd => {
                self.ui_state.overview.start_add();
            }
            OverviewAction::CancelAdd => {
                self.ui_state.overview.cancel_add();
            }
            OverviewAction::SaveAdd {
                endpoint,
                node,
                service,
            } => match self.state.register_agent(&endpoint, &node, &service) {
                Ok(_) => self.ui_state.overview.registered_agent(),
                Err(error) => self.ui_state.overview.rejected_agent(error),
            },
            OverviewAction::RequestRemove(key) => {
                self.ui_state.overview.request_remove(key);
            }
            OverviewAction::ConfirmRemove(key) => match self.state.remove_agent(&key) {
                Ok(()) => self.ui_state.overview.removed_agent(),
                Err(error) => self.ui_state.overview.rejected_remove(error),
            },
            OverviewAction::CancelRemove => {
                self.ui_state.overview.cancel_remove();
            }
            OverviewAction::SaveListener(addr) => match self.state.add_listener(&addr) {
                Ok(()) => self.ui_state.overview.bound_listener(),
                Err(error) => self.ui_state.overview.rejected_listener(error),
            },
            OverviewAction::RequestRemoveListener(addr) => {
                self.ui_state.overview.request_remove_listener(addr);
            }
            OverviewAction::ConfirmRemoveListener(addr) => {
                match self.state.remove_listener(&addr) {
                    Ok(()) => self.ui_state.overview.removed_listener(),
                    Err(error) => self.ui_state.overview.rejected_listener(error),
                }
            }
            OverviewAction::CancelRemoveListener => {
                self.ui_state.overview.cancel_remove_listener();
            }
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let alerts_enabled = self.state.alerts_enabled();
        let changes = self.ui_state.settings.show_window(
            ctx,
            &mut self.ui_state.settings_open,
            alerts_enabled,
        );
        self.apply_settings_changes(ctx, changes);
    }

    fn show_character_settings_window(&mut self, ctx: &egui::Context) {
        let changes = self.ui_state.settings.show_character_window(
            ctx,
            &mut self.ui_state.character_settings_open,
            self.state.avatar_profile_save_pending(),
            self.ui_state.avatar_model.requested_path(),
            self.ui_state.avatar_model.error(),
        );
        self.apply_settings_changes(ctx, changes);
    }

    fn apply_settings_changes(
        &mut self,
        ctx: &egui::Context,
        changes: crate::ui_settings::SettingsChanges,
    ) {
        if let Some(enabled) = changes.alerts_enabled {
            self.state.set_alerts_enabled(enabled);
        }
        if let Some(profile) = changes.avatar_profile {
            if let Err(error) = self.state.set_avatar_profile(profile) {
                self.ui_state.settings.reject_avatar_profile(error.clone());
                self.state
                    .push_settings_error(format!("character profile rejected: {error}"));
            } else {
                if self.state.avatar_profile_save_pending() {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
                if changes.preview_character {
                    self.ui_state.character_preview_open = true;
                }
            }
        }
    }

    fn show_character_preview(&mut self, ctx: &egui::Context) {
        let nodes = self.state.nodes().values().take(1).collect::<Vec<_>>();
        let input = avatar::AvatarPresenterInput::for_node(
            &nodes,
            self.state.alerts(),
            self.state.avatar_profile(),
        );
        let mut preview_open = self.ui_state.character_preview_open;
        egui::Window::new("Character preview")
            .id(egui::Id::new("character-preview-window"))
            .open(&mut preview_open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(420.0, 560.0))
            .min_size(egui::vec2(300.0, 360.0))
            .show(ctx, |ui| {
                let panel_width = ui.available_width().clamp(280.0, 520.0);
                let panel_height = ui.available_height().clamp(320.0, 620.0);
                avatar::show(
                    ui,
                    panel_width,
                    panel_height,
                    input,
                    &self.ui_state.avatar_model,
                );
            });
        self.ui_state.character_preview_open = preview_open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_character_profile_opens_preview_on_startup() {
        let ctx = egui::Context::default();
        let mut profile = AvatarReactionProfile::default();
        profile.model_path = "/tmp/operator.vrm".to_string();

        let ui_state = ControlRoomUiState::new(&ctx, &profile, 0, 0);

        assert!(ui_state.character_preview_open);
    }

    #[test]
    fn built_in_character_does_not_force_preview_on_startup() {
        let ctx = egui::Context::default();
        let ui_state = ControlRoomUiState::new(&ctx, &AvatarReactionProfile::default(), 0, 0);

        assert!(!ui_state.character_preview_open);
    }
}
