use crate::components::{
    counters, header,
    layout::{
        ContentLayout, LayoutMode, PanelLimits, centered_content, remaining_height, section_gap,
    },
};
use crate::config;
use crate::pages::{
    detail,
    overview::{self, OverviewAction, OverviewState},
};
use crate::state::DashboardState;
use crate::ui_settings::UiSettings;
use eframe::egui;

pub(crate) struct ControlRoomUiState {
    selected_node_key: Option<String>,
    overview: OverviewState,
    settings_open: bool,
    show_avatar: bool,
    settings: UiSettings,
}

impl ControlRoomUiState {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let settings = UiSettings::default();
        settings.apply_visuals(ctx);

        Self {
            selected_node_key: None,
            overview: OverviewState::default(),
            settings_open: false,
            show_avatar: false,
            settings,
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
        self.ui_state.settings.load_dropped_image(ui.ctx());
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

                            if header::show(
                                ui,
                                compact,
                                self.state.status(),
                                self.state.alert_summary(),
                                self.state.operational_summary(),
                            ) {
                                self.ui_state.settings_open = true;
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
        if let Some(enabled) = changes.alerts_enabled {
            self.state.set_alerts_enabled(enabled);
        }
    }
}
