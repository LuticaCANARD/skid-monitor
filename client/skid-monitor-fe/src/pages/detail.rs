mod action;

pub(crate) use action::DetailAction;

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
    show_avatar: &mut bool,
) -> Option<DetailAction> {
    let mut action = None;
    let Some(node) = state.nodes().get(key) else {
        return None;
    };

    if matches!(
        show_toolbar(ui, node, show_avatar),
        Some(DetailAction::BackToOverview)
    ) {
        action = Some(DetailAction::BackToOverview);
    }
    ui.add_space(config::SECTION_GAP);

    if let Some(data) = detail_page_data(state, key, *show_avatar) {
        show_resizable_content(ui, compact, panel_width, layout, limits, data);
    }

    action
}

fn show_toolbar(
    ui: &mut egui::Ui,
    node: &NodeSummary,
    show_avatar: &mut bool,
) -> Option<DetailAction> {
    #[cfg(not(feature = "high-spec"))]
    let _ = show_avatar;
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

        #[cfg(feature = "high-spec")]
        {
            ui.separator();
            ui.selectable_value(show_avatar, false, "Metrics");
            ui.selectable_value(show_avatar, true, "Character");
        }
        ui.label(
            RichText::new(format!("via {} / {}", node.endpoint, node.service))
                .monospace()
                .color(ui.visuals().weak_text_color()),
        );
    });

    action
}

fn detail_page_data<'a>(
    state: &'a DashboardState,
    key: &str,
    show_avatar: bool,
) -> Option<DetailPageData<'a>> {
    #[cfg(not(feature = "high-spec"))]
    let _ = show_avatar;
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
        #[cfg(feature = "high-spec")]
        show_avatar,
    );

    Some(DetailPageData { panels, events })
}

fn show_resizable_content(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    layout: LayoutMode,
    limits: PanelLimits,
    data: DetailPageData<'_>,
) {
    let gap = crate::components::layout::section_gap(ui);
    let main_min = main_panels::minimum_height(layout, gap);
    let event_min = config::EVENT_LOG_HEIGHT_MIN;
    let available = (limits.main_height + limits.event_log_height).max(main_min + event_min);
    let id = ui.make_persistent_id("detail-main-event-flex");
    let mut flex = ui.ctx().data_mut(|data| {
        data.get_temp::<[f32; 2]>(id).unwrap_or([
            limits.main_height.max(main_min),
            limits.event_log_height.max(event_min),
        ])
    });
    let total = flex[0] + flex[1];
    let mut main_height =
        (available * flex[0] / total.max(f32::EPSILON)).clamp(main_min, available - event_min);
    let mut event_height = available - main_height;

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        main_panels::show(
            ui,
            compact,
            panel_width,
            layout,
            PanelLimits {
                main_height,
                metrics_height: main_height,
                ..limits
            },
            data.panels,
        );

        let delta = crate::components::layout::vertical_resize_handle(ui, panel_width, gap);
        event_log::show(ui, panel_width, event_height, &data.events);

        if delta != 0.0 {
            main_height = (main_height + delta).clamp(main_min, available - event_min);
            event_height = available - main_height;
            flex = [main_height, event_height];
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, flex));
}

fn sample_matches_key(sample: &MetricSample, key: &str) -> bool {
    edge_key(&sample.endpoint, &sample.node) == key
}

fn event_matches_node(event: &EventRow, node: &NodeSummary) -> bool {
    event.message.contains(&node.node) || event.message.contains(&node.endpoint)
}
