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

pub(crate) enum DetailAction {
    BackToOverview,
}

struct DetailPageData<'a> {
    panels: MainPanelData<'a>,
    events: Vec<&'a EventRow>,
}

pub(crate) fn selected_key(selected_key: Option<&str>, state: &DashboardState) -> Option<String> {
    selected_key
        .filter(|key| state.nodes().contains_key(*key))
        .map(str::to_owned)
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    layout: LayoutMode,
    limits: PanelLimits,
    state: &DashboardState,
    key: &str,
) -> Option<DetailAction> {
    let mut action = None;
    let Some(node) = state.nodes().get(key) else {
        return None;
    };

    if matches!(show_toolbar(ui, node), Some(DetailAction::BackToOverview)) {
        action = Some(DetailAction::BackToOverview);
    }
    ui.add_space(config::SECTION_GAP);

    if let Some(data) = detail_page_data(state, key) {
        main_panels::show(ui, compact, panel_width, layout, limits, data.panels);
        ui.add_space(config::SECTION_GAP);
        event_log::show(ui, panel_width, limits.event_log_height, &data.events);
    }

    action
}

fn show_toolbar(ui: &mut egui::Ui, node: &NodeSummary) -> Option<DetailAction> {
    let mut action = None;

    ui.horizontal_wrapped(|ui| {
        if ui.button("Agents").clicked() {
            action = Some(DetailAction::BackToOverview);
        }
        ui.label(
            RichText::new(&node.node)
                .strong()
                .color(ui.visuals().strong_text_color()),
        );
        ui.label(
            RichText::new(format!("via {} / {}", node.endpoint, node.service))
                .monospace()
                .color(ui.visuals().weak_text_color()),
        );
    });

    action
}

fn detail_page_data<'a>(state: &'a DashboardState, key: &str) -> Option<DetailPageData<'a>> {
    let node = state.nodes().get(key)?;
    let metrics = state
        .metrics()
        .iter()
        .filter(|sample| sample_matches_key(sample, key))
        .collect::<Vec<_>>();
    let events = state
        .events()
        .iter()
        .filter(|event| event_matches_node(event, node))
        .collect::<Vec<_>>();
    let panels = MainPanelData::new(
        vec![node],
        state.edge_decorations(),
        metrics,
        state.metric_history(),
        state.alerts(),
    );

    Some(DetailPageData { panels, events })
}

fn sample_matches_key(sample: &MetricSample, key: &str) -> bool {
    edge_key(&sample.endpoint, &sample.node) == key
}

fn event_matches_node(event: &EventRow, node: &NodeSummary) -> bool {
    event.message.contains(&node.node) || event.message.contains(&node.endpoint)
}
