use crate::alert::AlertStore;
use crate::components::{
    layout::{LayoutMode, PanelLimits, graph_panel_width},
    metrics, nodes, trends,
};
use crate::config;
use crate::edge::EdgeSignalDecorations;
use crate::model::{MetricSample, NodeSummary};
use eframe::egui;
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct MainPanelData<'a> {
    nodes: Vec<&'a NodeSummary>,
    edge_decorations: &'a EdgeSignalDecorations,
    metrics: Vec<&'a MetricSample>,
    metric_history: &'a BTreeMap<String, VecDeque<f64>>,
    alerts: &'a AlertStore,
}

impl<'a> MainPanelData<'a> {
    pub(crate) fn new(
        nodes: Vec<&'a NodeSummary>,
        edge_decorations: &'a EdgeSignalDecorations,
        metrics: Vec<&'a MetricSample>,
        metric_history: &'a BTreeMap<String, VecDeque<f64>>,
        alerts: &'a AlertStore,
    ) -> Self {
        Self {
            nodes,
            edge_decorations,
            metrics,
            metric_history,
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
        data: &MainPanelData<'_>,
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
        data: &MainPanelData<'_>,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        panel_height: f32,
    ) {
        match self {
            Self::Nodes => nodes::show(
                ui,
                compact,
                &data.nodes,
                data.edge_decorations,
                panel_width,
                panel_height,
            ),
            Self::Trends => trends::show(
                ui,
                compact,
                panel_width,
                panel_height,
                &data.metrics,
                data.metric_history,
            ),
            Self::Metrics => metrics::show(
                ui,
                compact,
                panel_width,
                panel_height,
                &data.metrics,
                data.alerts,
            ),
        }
    }
}

const STACKED_PANELS: [MainPanel; 3] = [MainPanel::Nodes, MainPanel::Trends, MainPanel::Metrics];
const GRAPH_PANELS: [MainPanel; 2] = [MainPanel::Nodes, MainPanel::Trends];

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    layout: LayoutMode,
    limits: PanelLimits,
    data: MainPanelData<'_>,
) {
    match layout {
        LayoutMode::Split => split(ui, compact, panel_width, limits, &data),
        LayoutMode::Stacked | LayoutMode::Compact => {
            stack(ui, compact, panel_width, limits, &data, &STACKED_PANELS);
        }
    }
}

fn split(
    ui: &mut egui::Ui,
    compact: bool,
    content_width: f32,
    limits: PanelLimits,
    data: &MainPanelData<'_>,
) {
    let spacing = ui.spacing().item_spacing.x;
    let graph_width = graph_panel_width(content_width);
    let metrics_width =
        (content_width - graph_width - spacing).max(config::METRICS_TABLE_MIN_WIDTH);

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(graph_width);
            stack(ui, compact, graph_width, limits, data, &GRAPH_PANELS);
        });
        ui.vertical(|ui| {
            ui.set_width(metrics_width);
            panel(ui, compact, metrics_width, limits, data, MainPanel::Metrics);
        });
    });
}

fn stack(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    limits: PanelLimits,
    data: &MainPanelData<'_>,
    panels: &[MainPanel],
) {
    for (index, panel_kind) in panels.iter().copied().enumerate() {
        if index > 0 {
            ui.add_space(config::SECTION_GAP);
        }
        panel(ui, compact, panel_width, limits, data, panel_kind);
    }
}

fn panel(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    limits: PanelLimits,
    data: &MainPanelData<'_>,
    panel_kind: MainPanel,
) {
    panel_kind.render(data, ui, compact, panel_width, panel_kind.height(limits));
}
