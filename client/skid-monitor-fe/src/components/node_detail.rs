use crate::components::{
    event_log,
    layout::{LayoutMode, PanelLimits},
    main_panels::{self, MainPanelData},
};
use crate::config;
use crate::edge::edge_key;
use crate::model::{EventRow, MetricSample, NodeSummary};
use crate::state::DashboardState;
use eframe::egui::{self, RichText};
use std::collections::{BTreeMap, VecDeque};

pub(crate) enum DetailToolbarAction {
    BackToAgents,
}

struct FilteredPanelData {
    nodes: BTreeMap<String, NodeSummary>,
    metrics: VecDeque<MetricSample>,
    metric_history: BTreeMap<String, VecDeque<f64>>,
    events: VecDeque<EventRow>,
}

pub(crate) fn selected_key(selected_key: Option<&str>, state: &DashboardState) -> Option<String> {
    selected_key
        .filter(|key| state.nodes().contains_key(*key))
        .map(str::to_owned)
}

pub(crate) fn show_toolbar(ui: &mut egui::Ui, node: &NodeSummary) -> Option<DetailToolbarAction> {
    let mut action = None;

    ui.horizontal_wrapped(|ui| {
        if ui.button("Agents").clicked() {
            action = Some(DetailToolbarAction::BackToAgents);
        }
        ui.label(
            RichText::new(&node.node)
                .strong()
                .color(ui.visuals().strong_text_color()),
        );
        ui.label(
            RichText::new(format!("{} / {}", node.endpoint, node.service))
                .monospace()
                .color(ui.visuals().weak_text_color()),
        );
    });

    action
}

pub(crate) fn show_body(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    layout: LayoutMode,
    limits: PanelLimits,
    state: &DashboardState,
    key: &str,
) {
    let Some(filtered) = filtered_panel_data(state, key) else {
        return;
    };
    let data = MainPanelData::new(
        &filtered.nodes,
        state.edge_decorations(),
        &filtered.metrics,
        &filtered.metric_history,
        state.alerts(),
    );

    main_panels::show(ui, compact, panel_width, layout, limits, data);
    ui.add_space(config::SECTION_GAP);
    event_log::show(ui, panel_width, limits.event_log_height, &filtered.events);
}

fn filtered_panel_data(state: &DashboardState, key: &str) -> Option<FilteredPanelData> {
    let node = state.nodes().get(key)?;
    let mut nodes = BTreeMap::new();
    nodes.insert(key.to_string(), node.clone());

    let metrics = state
        .metrics()
        .iter()
        .filter(|sample| edge_key(&sample.endpoint, &sample.node) == key)
        .cloned()
        .collect::<VecDeque<_>>();

    let mut metric_history = BTreeMap::new();
    for sample in &metrics {
        if let Some(values) = state.metric_history().get(&sample.trend_key) {
            metric_history.insert(sample.trend_key.clone(), values.clone());
        }
    }

    let events = state
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

fn event_matches_node(event: &EventRow, node: &NodeSummary) -> bool {
    event.message.contains(&node.node) || event.message.contains(&node.endpoint)
}
