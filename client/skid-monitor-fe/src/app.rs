use crate::alert::AlertStore;
use crate::components::{
    agents::{self, AddAgentDraft, AgentNotice, AgentOverviewAction},
    counters, event_log, header,
    layout::{
        ContentLayout, LayoutMode, PanelLimits, centered_content, graph_panel_width,
        remaining_height, section_gap,
    },
    metrics, nodes, trends,
};
use crate::config;
use crate::edge::{EdgeSignalDecorations, edge_key};
use crate::model::{EventRow, MetricSample, NodeSummary};
use crate::state::DashboardState;
use eframe::egui::{self, RichText};
use skid_monitor_client::receiver_loop::{ReceiverMessage, spawn_receiver_with_notify};
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;

pub(crate) struct ControlRoomApp {
    rx: Receiver<ReceiverMessage>,
    state: DashboardState,
    selected_node_key: Option<String>,
    add_agent_open: bool,
    add_agent_draft: AddAgentDraft,
    add_agent_notice: Option<AddAgentNotice>,
}

struct AddAgentNotice {
    message: String,
    is_error: bool,
}

#[derive(Clone, Copy)]
struct PanelData<'a> {
    nodes: &'a BTreeMap<String, NodeSummary>,
    edge_decorations: &'a EdgeSignalDecorations,
    metrics: &'a VecDeque<MetricSample>,
    metric_history: &'a BTreeMap<String, VecDeque<f64>>,
    alerts: &'a AlertStore,
}

struct FilteredPanelData {
    nodes: BTreeMap<String, NodeSummary>,
    metrics: VecDeque<MetricSample>,
    metric_history: BTreeMap<String, VecDeque<f64>>,
    events: VecDeque<EventRow>,
}

impl FilteredPanelData {
    fn panel_data<'a>(
        &'a self,
        edge_decorations: &'a EdgeSignalDecorations,
        alerts: &'a AlertStore,
    ) -> PanelData<'a> {
        PanelData {
            nodes: &self.nodes,
            edge_decorations,
            metrics: &self.metrics,
            metric_history: &self.metric_history,
            alerts,
        }
    }
}

#[derive(Clone, Copy)]
enum MainPanel {
    Nodes,
    Trends,
    Metrics,
}

trait PanelTemplate {
    fn height(self, limits: PanelLimits) -> f32;

    fn render(
        self,
        data: PanelData<'_>,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        panel_height: f32,
    );
}

impl PanelTemplate for MainPanel {
    fn height(self, limits: PanelLimits) -> f32 {
        match self {
            Self::Nodes => limits.sources_height,
            Self::Trends => limits.trends_height,
            Self::Metrics => limits.metrics_height,
        }
    }

    fn render(
        self,
        data: PanelData<'_>,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        panel_height: f32,
    ) {
        match self {
            Self::Nodes => nodes::show(
                ui,
                compact,
                data.nodes,
                data.edge_decorations,
                panel_width,
                panel_height,
            ),
            Self::Trends => trends::show(
                ui,
                compact,
                panel_width,
                panel_height,
                data.metrics,
                data.metric_history,
            ),
            Self::Metrics => metrics::show(
                ui,
                compact,
                panel_width,
                panel_height,
                data.metrics,
                data.alerts,
            ),
        }
    }
}

const STACKED_MAIN_PANELS: [MainPanel; 3] =
    [MainPanel::Nodes, MainPanel::Trends, MainPanel::Metrics];
const GRAPH_MAIN_PANELS: [MainPanel; 2] = [MainPanel::Nodes, MainPanel::Trends];

impl ControlRoomApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.global_style_mut(|style| {
            style.spacing.item_spacing = config::GLOBAL_ITEM_SPACING;
            style.spacing.button_padding = config::GLOBAL_BUTTON_PADDING;
        });

        let ctx = cc.egui_ctx.clone();

        Self {
            rx: spawn_receiver_with_notify(move || ctx.request_repaint()),
            state: DashboardState::new(),
            selected_node_key: None,
            add_agent_open: false,
            add_agent_draft: AddAgentDraft::default(),
            add_agent_notice: None,
        }
    }

    fn main_stack(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
        data: PanelData<'_>,
    ) {
        self.panel_stack(ui, compact, panel_width, limits, data, &STACKED_MAIN_PANELS);
    }

    fn main_split(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        content_width: f32,
        limits: PanelLimits,
        data: PanelData<'_>,
    ) {
        let spacing = ui.spacing().item_spacing.x;
        let graph_width = graph_panel_width(content_width);
        let metrics_width =
            (content_width - graph_width - spacing).max(config::METRICS_TABLE_MIN_WIDTH);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(graph_width);
                self.panel_stack(ui, compact, graph_width, limits, data, &GRAPH_MAIN_PANELS);
            });
            ui.vertical(|ui| {
                ui.set_width(metrics_width);
                self.panel(ui, compact, metrics_width, limits, data, MainPanel::Metrics);
            });
        });
    }

    fn panel_stack(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
        data: PanelData<'_>,
        panels: &[MainPanel],
    ) {
        for (index, panel) in panels.iter().copied().enumerate() {
            if index > 0 {
                ui.add_space(config::SECTION_GAP);
            }
            self.panel(ui, compact, panel_width, limits, data, panel);
        }
    }

    fn panel(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
        data: PanelData<'_>,
        panel: MainPanel,
    ) {
        panel.render(data, ui, compact, panel_width, panel.height(limits));
    }

    fn current_selected_key(&self) -> Option<String> {
        self.selected_node_key
            .clone()
            .filter(|key| self.state.nodes().contains_key(key))
    }

    fn detail_toolbar(&mut self, ui: &mut egui::Ui, key: &str) {
        let Some(node) = self.state.nodes().get(key) else {
            return;
        };

        ui.horizontal_wrapped(|ui| {
            if ui.button("Agents").clicked() {
                self.selected_node_key = None;
            }
            ui.label(
                RichText::new(&node.node)
                    .strong()
                    .color(config::TITLE_COLOR),
            );
            ui.label(
                RichText::new(format!("{} / {}", node.endpoint, node.service))
                    .monospace()
                    .color(config::MUTED_TEXT_COLOR),
            );
        });
    }

    fn show_overview(
        &mut self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
    ) {
        let notice = self.add_agent_notice.as_ref().map(|notice| AgentNotice {
            message: notice.message.as_str(),
            is_error: notice.is_error,
        });
        let action = agents::show(
            ui,
            compact,
            self.state.nodes(),
            self.state.edge_decorations(),
            &mut self.add_agent_draft,
            self.add_agent_open,
            notice,
            panel_width,
            limits.main_height,
        );

        if let Some(action) = action {
            self.handle_agent_action(action);
        }
    }

    fn show_node_detail(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        layout: LayoutMode,
        limits: PanelLimits,
        key: &str,
    ) {
        let Some(filtered) = self.filtered_panel_data(key) else {
            return;
        };
        let data = filtered.panel_data(self.state.edge_decorations(), self.state.alerts());

        match layout {
            LayoutMode::Split => {
                self.main_split(ui, compact, panel_width, limits, data);
            }
            LayoutMode::Stacked | LayoutMode::Compact => {
                self.main_stack(ui, compact, panel_width, limits, data);
            }
        }
        ui.add_space(config::SECTION_GAP);
        event_log::show(ui, panel_width, limits.event_log_height, &filtered.events);
    }

    fn handle_agent_action(&mut self, action: AgentOverviewAction) {
        match action {
            AgentOverviewAction::Select(key) => {
                self.selected_node_key = Some(key);
                self.add_agent_notice = None;
            }
            AgentOverviewAction::StartAdd => {
                self.add_agent_open = true;
                self.add_agent_notice = None;
            }
            AgentOverviewAction::CancelAdd => {
                self.add_agent_open = false;
                self.add_agent_notice = None;
                self.add_agent_draft.clear();
            }
            AgentOverviewAction::SaveAdd {
                endpoint,
                node,
                service,
            } => match self.state.register_agent(&endpoint, &node, &service) {
                Ok(_) => {
                    self.add_agent_open = false;
                    self.add_agent_draft.clear();
                    self.add_agent_notice = Some(AddAgentNotice {
                        message: "agent registered".to_string(),
                        is_error: false,
                    });
                }
                Err(error) => {
                    self.add_agent_open = true;
                    self.add_agent_notice = Some(AddAgentNotice {
                        message: error,
                        is_error: true,
                    });
                }
            },
        }
    }

    fn filtered_panel_data(&self, key: &str) -> Option<FilteredPanelData> {
        let node = self.state.nodes().get(key)?;
        let mut nodes = BTreeMap::new();
        nodes.insert(key.to_string(), node.clone());

        let metrics = self
            .state
            .metrics()
            .iter()
            .filter(|sample| edge_key(&sample.endpoint, &sample.node) == key)
            .cloned()
            .collect::<VecDeque<_>>();

        let mut metric_history = BTreeMap::new();
        for sample in &metrics {
            if let Some(values) = self.state.metric_history().get(&sample.trend_key) {
                metric_history.insert(sample.trend_key.clone(), values.clone());
            }
        }

        let events = self
            .state
            .events()
            .iter()
            .filter(|event| event_matches_node(event, node))
            .cloned()
            .collect();

        Some(FilteredPanelData {
            nodes,
            metrics,
            metric_history,
            events,
        })
    }
}

fn event_matches_node(event: &EventRow, node: &NodeSummary) -> bool {
    event.message.contains(&node.node) || event.message.contains(&node.endpoint)
}

impl eframe::App for ControlRoomApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.state.drain_messages(&self.rx);

        egui::Frame::default()
            .fill(config::PAGE_BACKGROUND)
            .inner_margin(egui::Margin::same(config::CONTENT_FRAME_MARGIN))
            .show(ui, |ui| {
                let content = ContentLayout::for_viewport(ui.clip_rect().size());

                centered_content(ui, content, |ui| {
                    let panel_width = ui.available_width();
                    let layout = LayoutMode::for_width(panel_width);
                    let compact = layout.is_compact();

                    header::show(ui, compact, self.state.status(), self.state.alert_summary());
                    ui.add_space(config::HEADER_COUNTER_GAP);
                    counters::show(ui, self.state.counters());
                    ui.add_space(config::SECTION_GAP);

                    let selected_key = self.current_selected_key();
                    if self.selected_node_key != selected_key {
                        self.selected_node_key = selected_key.clone();
                    }
                    if let Some(key) = selected_key.as_deref() {
                        self.detail_toolbar(ui, key);
                        ui.add_space(config::SECTION_GAP);
                    }

                    let section_gap = section_gap(ui);
                    let limits = PanelLimits::for_remaining_height(
                        remaining_height(ui, content),
                        layout,
                        section_gap,
                    );
                    if let Some(key) = self.current_selected_key() {
                        self.show_node_detail(ui, compact, panel_width, layout, limits, &key);
                    } else {
                        self.show_overview(ui, compact, panel_width, limits);
                        ui.add_space(config::SECTION_GAP);
                        event_log::show(
                            ui,
                            panel_width,
                            limits.event_log_height,
                            self.state.events(),
                        );
                    }
                    ui.add_space(content.bottom_margin);
                });
            });
    }
}
