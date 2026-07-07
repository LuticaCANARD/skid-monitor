use crate::app::ControlRoomApp;
use crate::components::{
    agents::{self, AddAgentDraft, AgentNotice, AgentOverviewAction},
    counters, event_log, header,
    layout::{
        ContentLayout, LayoutMode, PanelLimits, centered_content, remaining_height, section_gap,
    },
    node_detail,
};
use crate::config;
use crate::ui_settings::UiSettings;
use eframe::egui;

pub(crate) struct AddAgentNotice {
    message: String,
    is_error: bool,
}

pub(crate) struct ControlRoomUiState {
    selected_node_key: Option<String>,
    add_agent_open: bool,
    add_agent_draft: AddAgentDraft,
    add_agent_notice: Option<AddAgentNotice>,
    settings_open: bool,
    settings: UiSettings,
}

impl ControlRoomUiState {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let settings = UiSettings::default();
        settings.apply_visuals(ctx);

        Self {
            selected_node_key: None,
            add_agent_open: false,
            add_agent_draft: AddAgentDraft::default(),
            add_agent_notice: None,
            settings_open: false,
            settings,
        }
    }
}

pub(crate) struct ControlRoomView<'a> {
    app: &'a mut ControlRoomApp,
}

impl<'a> ControlRoomView<'a> {
    pub(crate) fn new(app: &'a mut ControlRoomApp) -> Self {
        Self { app }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        self.app.ui.settings.load_dropped_image(ui.ctx());
        self.app.ui.settings.paint_background(ui);

        egui::Frame::default()
            .inner_margin(egui::Margin::same(config::CONTENT_FRAME_MARGIN))
            .show(ui, |ui| {
                let content = ContentLayout::for_viewport(ui.clip_rect().size());

                centered_content(ui, content, |ui| {
                    let panel_width = ui.available_width();
                    let layout = LayoutMode::for_width(panel_width);
                    let compact = layout.is_compact();

                    if header::show(
                        ui,
                        compact,
                        self.app.state.status(),
                        self.app.state.alert_summary(),
                    ) {
                        self.app.ui.settings_open = true;
                    }
                    ui.add_space(config::HEADER_COUNTER_GAP);
                    counters::show(ui, self.app.state.counters());
                    ui.add_space(config::SECTION_GAP);

                    let selected_key = self.current_selected_key();
                    if self.app.ui.selected_node_key != selected_key {
                        self.app.ui.selected_node_key = selected_key.clone();
                    }
                    if let Some(key) = selected_key.as_deref() {
                        self.show_detail_toolbar(ui, key);
                        ui.add_space(config::SECTION_GAP);
                    }

                    let section_gap = section_gap(ui);
                    let limits = PanelLimits::for_remaining_height(
                        remaining_height(ui, content),
                        layout,
                        section_gap,
                    );
                    if let Some(key) = self.current_selected_key() {
                        node_detail::show_body(
                            ui,
                            compact,
                            panel_width,
                            layout,
                            limits,
                            &self.app.state,
                            &key,
                        );
                    } else {
                        self.show_overview(ui, compact, panel_width, limits);
                        ui.add_space(config::SECTION_GAP);
                        event_log::show(
                            ui,
                            panel_width,
                            limits.event_log_height,
                            self.app.state.events(),
                        );
                    }
                    ui.add_space(content.bottom_margin);
                });
            });

        if self.app.ui.settings_open {
            self.show_settings_window(ui.ctx());
        }
    }

    fn current_selected_key(&self) -> Option<String> {
        node_detail::selected_key(self.app.ui.selected_node_key.as_deref(), &self.app.state)
    }

    fn show_overview(
        &mut self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
    ) {
        let notice = self
            .app
            .ui
            .add_agent_notice
            .as_ref()
            .map(|notice| AgentNotice {
                message: notice.message.as_str(),
                is_error: notice.is_error,
            });
        let action = agents::show(
            ui,
            compact,
            self.app.state.nodes(),
            self.app.state.edge_decorations(),
            &mut self.app.ui.add_agent_draft,
            self.app.ui.add_agent_open,
            notice,
            panel_width,
            limits.main_height,
        );

        if let Some(action) = action {
            self.handle_agent_action(action);
        }
    }

    fn show_detail_toolbar(&mut self, ui: &mut egui::Ui, key: &str) {
        let Some(node) = self.app.state.nodes().get(key) else {
            return;
        };

        if matches!(
            node_detail::show_toolbar(ui, node),
            Some(node_detail::DetailToolbarAction::BackToAgents)
        ) {
            self.app.ui.selected_node_key = None;
        }
    }

    fn handle_agent_action(&mut self, action: AgentOverviewAction) {
        match action {
            AgentOverviewAction::Select(key) => {
                self.app.ui.selected_node_key = Some(key);
                self.app.ui.add_agent_notice = None;
            }
            AgentOverviewAction::StartAdd => {
                self.app.ui.add_agent_open = true;
                self.app.ui.add_agent_notice = None;
            }
            AgentOverviewAction::CancelAdd => {
                self.app.ui.add_agent_open = false;
                self.app.ui.add_agent_notice = None;
                self.app.ui.add_agent_draft.clear();
            }
            AgentOverviewAction::SaveAdd {
                endpoint,
                node,
                service,
            } => match self.app.state.register_agent(&endpoint, &node, &service) {
                Ok(_) => {
                    self.app.ui.add_agent_open = false;
                    self.app.ui.add_agent_draft.clear();
                    self.app.ui.add_agent_notice = Some(AddAgentNotice {
                        message: "agent registered".to_string(),
                        is_error: false,
                    });
                }
                Err(error) => {
                    self.app.ui.add_agent_open = true;
                    self.app.ui.add_agent_notice = Some(AddAgentNotice {
                        message: error,
                        is_error: true,
                    });
                }
            },
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let alerts_enabled = self.app.state.alerts_enabled();
        let changes =
            self.app
                .ui
                .settings
                .show_window(ctx, &mut self.app.ui.settings_open, alerts_enabled);
        if let Some(enabled) = changes.alerts_enabled {
            self.app.state.set_alerts_enabled(enabled);
        }
    }
}
