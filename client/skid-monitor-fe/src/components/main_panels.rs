mod kind;

use crate::alert::AlertStore;
use crate::components::{
    avatar, database_metrics,
    layout::{LayoutMode, PanelLimits, graph_panel_width},
    metrics, nodes, trends,
};
use crate::config;
use crate::edge::EdgeSignalDecorations;
use crate::model::{AvatarReactionProfile, MetricSample, NodeSummary};
use eframe::egui;
use kind::MainPanel;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Default)]
struct PanelFlex {
    split: Option<[f32; 2]>,
    graph_stack: Option<[f32; 3]>,
    full_stack: Option<[f32; 4]>,
}

pub(crate) struct MainPanelData<'a> {
    nodes: Vec<&'a NodeSummary>,
    edge_decorations: &'a EdgeSignalDecorations,
    metrics: Vec<&'a MetricSample>,
    metric_history: &'a BTreeMap<String, VecDeque<f64>>,
    alerts: &'a AlertStore,
    character: CharacterPanelData<'a>,
}

pub(crate) struct CharacterPanelData<'a> {
    pub(crate) profile: &'a AvatarReactionProfile,
    pub(crate) model: &'a avatar::AvatarModelCache,
    pub(crate) visible: bool,
}

impl<'a> MainPanelData<'a> {
    pub(crate) fn new(
        nodes: Vec<&'a NodeSummary>,
        edge_decorations: &'a EdgeSignalDecorations,
        metrics: Vec<&'a MetricSample>,
        metric_history: &'a BTreeMap<String, VecDeque<f64>>,
        alerts: &'a AlertStore,
        character: CharacterPanelData<'a>,
    ) -> Self {
        Self {
            nodes,
            edge_decorations,
            metrics,
            metric_history,
            alerts,
            character,
        }
    }
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
            Self::Database => limits.database_height,
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
            Self::Database => {
                database_metrics::show(ui, compact, panel_width, panel_height, &data.metrics)
            }
            Self::Trends => trends::show(
                ui,
                compact,
                panel_width,
                panel_height,
                &data.metrics,
                data.metric_history,
            ),
            Self::Metrics => {
                if data.character.visible {
                    avatar::show(
                        ui,
                        panel_width,
                        panel_height,
                        avatar::AvatarPresenterInput::for_node(
                            &data.nodes,
                            data.alerts,
                            data.character.profile,
                        ),
                        data.character.model,
                    );
                    return;
                }

                metrics::show(
                    ui,
                    compact,
                    panel_width,
                    panel_height,
                    &data.metrics,
                    data.alerts,
                );
            }
        }
    }
}

impl MainPanel {
    fn minimum_height(self) -> f32 {
        match self {
            Self::Nodes => config::SOURCES_HEIGHT_MIN,
            Self::Database => config::DATABASE_METRICS_HEIGHT_MIN,
            Self::Trends => config::TRENDS_PANEL_HEIGHT_MIN,
            Self::Metrics => config::METRICS_TABLE_HEIGHT_MIN,
        }
    }
}

const STACKED_PANELS: [MainPanel; 4] = [
    MainPanel::Nodes,
    MainPanel::Database,
    MainPanel::Trends,
    MainPanel::Metrics,
];
const GRAPH_PANELS: [MainPanel; 3] = [MainPanel::Nodes, MainPanel::Database, MainPanel::Trends];

pub(crate) fn minimum_height(layout: LayoutMode, gap: f32) -> f32 {
    let panels = match layout {
        LayoutMode::Split => GRAPH_PANELS.as_slice(),
        LayoutMode::Stacked | LayoutMode::Compact => STACKED_PANELS.as_slice(),
    };
    let stacked_height = panels
        .iter()
        .map(|panel| panel.minimum_height())
        .sum::<f32>()
        + gap * panels.len().saturating_sub(1) as f32;

    if matches!(layout, LayoutMode::Split) {
        stacked_height.max(MainPanel::Metrics.minimum_height())
    } else {
        stacked_height
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    layout: LayoutMode,
    limits: PanelLimits,
    data: MainPanelData<'_>,
) {
    let flex_id = ui.make_persistent_id("main-panel-flex");
    let mut flex = ui
        .ctx()
        .data_mut(|data| data.get_temp::<PanelFlex>(flex_id).unwrap_or_default());

    match layout {
        LayoutMode::Split => split(ui, compact, panel_width, limits, &data, &mut flex),
        LayoutMode::Stacked | LayoutMode::Compact => {
            let initial = panel_heights(limits, &STACKED_PANELS);
            let weights = flex.full_stack.get_or_insert(initial);
            stack(
                ui,
                compact,
                panel_width,
                limits.main_height,
                &data,
                &STACKED_PANELS,
                weights,
            );
        }
    }

    ui.ctx().data_mut(|data| data.insert_temp(flex_id, flex));
}

fn split(
    ui: &mut egui::Ui,
    compact: bool,
    content_width: f32,
    limits: PanelLimits,
    data: &MainPanelData<'_>,
    flex: &mut PanelFlex,
) {
    let handle_width = ui.spacing().item_spacing.x;
    let content_spacing_x = ui.spacing().item_spacing.x;
    let column_budget = (content_width - handle_width).max(1.0);
    let initial_graph_width = graph_panel_width(content_width).min(column_budget);
    let columns = flex
        .split
        .get_or_insert([initial_graph_width, column_budget - initial_graph_width]);
    normalize_flex(columns);
    let mut widths = flex_extents(columns, column_budget);
    clamp_pair(
        &mut widths,
        0,
        0.0,
        config::GRAPH_PANEL_MIN_WIDTH.min(column_budget),
        config::METRICS_TABLE_MIN_WIDTH.min(column_budget),
    );

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.x = content_spacing_x;
            ui.set_width(widths[0]);
            let initial = panel_heights(limits, &GRAPH_PANELS);
            let weights = flex.graph_stack.get_or_insert(initial);
            stack(
                ui,
                compact,
                widths[0],
                limits.main_height,
                data,
                &GRAPH_PANELS,
                weights,
            );
        });
        let delta = resize_handle(ui, egui::vec2(handle_width, limits.main_height), true);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.x = content_spacing_x;
            ui.set_width(widths[1]);
            panel(
                ui,
                compact,
                widths[1],
                limits.metrics_height,
                data,
                MainPanel::Metrics,
            );
        });

        if delta != 0.0 {
            clamp_pair(
                &mut widths,
                0,
                delta,
                config::GRAPH_PANEL_MIN_WIDTH.min(column_budget),
                config::METRICS_TABLE_MIN_WIDTH.min(column_budget),
            );
            *columns = widths;
        }
    });
}

fn stack<const N: usize>(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    total_height: f32,
    data: &MainPanelData<'_>,
    panels: &[MainPanel; N],
    flex: &mut [f32; N],
) {
    let content_spacing_y = ui.spacing().item_spacing.y;
    let gap = config::SECTION_GAP + content_spacing_y;
    let panel_budget = (total_height - gap * (N.saturating_sub(1) as f32)).max(1.0);
    normalize_flex(flex);
    let minimums = std::array::from_fn(|index| panels[index].minimum_height());
    let mut heights = flex_extents_with_minimums(flex, panel_budget, minimums);

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, panel_kind) in panels.iter().copied().enumerate() {
            if index > 0 {
                let delta = resize_handle(ui, egui::vec2(panel_width, gap), false);
                if delta != 0.0 {
                    clamp_pair(
                        &mut heights,
                        index - 1,
                        delta,
                        minimums[index - 1].min(panel_budget),
                        minimums[index].min(panel_budget),
                    );
                    *flex = heights;
                }
            }
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = content_spacing_y;
                panel(ui, compact, panel_width, heights[index], data, panel_kind);
            });
        }
    });
}

fn panel(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    panel_height: f32,
    data: &MainPanelData<'_>,
    panel_kind: MainPanel,
) {
    panel_kind.render(data, ui, compact, panel_width, panel_height);
}

fn panel_heights<const N: usize>(limits: PanelLimits, panels: &[MainPanel; N]) -> [f32; N] {
    std::array::from_fn(|index| panels[index].height(limits))
}

fn normalize_flex<const N: usize>(flex: &mut [f32; N]) {
    let total = flex.iter().copied().sum::<f32>();
    if total <= f32::EPSILON {
        flex.fill(1.0);
    }
}

fn flex_extents<const N: usize>(flex: &[f32; N], available: f32) -> [f32; N] {
    let total = flex.iter().copied().sum::<f32>().max(f32::EPSILON);
    std::array::from_fn(|index| available * flex[index] / total)
}

fn flex_extents_with_minimums<const N: usize>(
    flex: &[f32; N],
    available: f32,
    mut minimums: [f32; N],
) -> [f32; N] {
    if N == 0 {
        return [0.0; N];
    }

    let minimum_total = minimums.iter().sum::<f32>();
    if minimum_total > available && minimum_total > f32::EPSILON {
        let scale = available / minimum_total;
        for minimum in &mut minimums {
            *minimum *= scale;
        }
    }
    let mut extents = [0.0; N];
    let mut flexible = [true; N];
    let weights: [f32; N] = std::array::from_fn(|index| flex[index].max(f32::EPSILON));
    let mut remaining = available;

    loop {
        let weight_total = (0..N)
            .filter(|&index| flexible[index])
            .map(|index| weights[index])
            .sum::<f32>();
        let undersized = (0..N).find(|&index| {
            flexible[index] && remaining * weights[index] / weight_total < minimums[index]
        });
        let Some(index) = undersized else {
            for index in 0..N {
                if flexible[index] {
                    extents[index] = remaining * weights[index] / weight_total;
                }
            }
            break;
        };
        flexible[index] = false;
        extents[index] = minimums[index];
        remaining = (remaining - minimums[index]).max(0.0);
    }

    extents
}

fn clamp_pair<const N: usize>(
    extents: &mut [f32; N],
    first: usize,
    delta: f32,
    first_min: f32,
    second_min: f32,
) {
    let pair_total = extents[first] + extents[first + 1];
    let min_total = first_min + second_min;
    let (first_min, second_min) = if min_total > pair_total && min_total > f32::EPSILON {
        let scale = pair_total / min_total;
        (first_min * scale, second_min * scale)
    } else {
        (first_min, second_min)
    };
    let first_max = pair_total - second_min;
    let first_extent = (extents[first] + delta).clamp(first_min, first_max);
    extents[first] = first_extent;
    extents[first + 1] = pair_total - first_extent;
}

fn resize_handle(ui: &mut egui::Ui, size: egui::Vec2, horizontal: bool) -> f32 {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::drag());
    let response = response.on_hover_cursor(if horizontal {
        egui::CursorIcon::ResizeHorizontal
    } else {
        egui::CursorIcon::ResizeVertical
    });
    let color = if response.dragged() || response.hovered() {
        ui.visuals().widgets.hovered.fg_stroke.color
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    if horizontal {
        ui.painter().vline(
            rect.center().x,
            rect.y_range(),
            egui::Stroke::new(config::RESIZE_HANDLE_STROKE, color),
        );
    } else {
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(config::RESIZE_HANDLE_STROKE, color),
        );
    }

    if response.dragged() {
        ui.input(|input| {
            if horizontal {
                input.pointer.delta().x
            } else {
                input.pointer.delta().y
            }
        })
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(left: f32, right: f32) {
        assert!(
            (left - right).abs() < 0.001,
            "expected {left} to match {right}"
        );
    }

    #[test]
    fn flex_extents_preserve_the_available_space() {
        let extents = flex_extents(&[1.0, 2.0, 1.0], 400.0);

        assert_eq!(extents, [100.0, 200.0, 100.0]);
        assert_close(extents.iter().sum(), 400.0);
    }

    #[test]
    fn pair_resize_changes_only_adjacent_flex_items() {
        let mut extents = [100.0, 200.0, 300.0];

        clamp_pair(&mut extents, 0, 40.0, 64.0, 64.0);

        assert_eq!(extents, [140.0, 160.0, 300.0]);
    }

    #[test]
    fn pair_minimums_shrink_to_fit_a_small_budget() {
        let mut extents = [20.0, 30.0];

        clamp_pair(&mut extents, 0, 100.0, 64.0, 64.0);

        assert_close(extents[0], 25.0);
        assert_close(extents[1], 25.0);
    }

    #[test]
    fn flex_extents_keep_every_panel_usable_at_the_minimum_budget() {
        let extents = flex_extents_with_minimums(&[88.0, 72.0, 0.0], 304.0, [88.0, 108.0, 108.0]);

        assert_eq!(extents, [88.0, 108.0, 108.0]);
        assert_close(extents.iter().sum(), 304.0);
    }

    #[test]
    fn split_minimum_height_covers_the_full_left_stack() {
        let gap = 20.0;

        assert_close(
            minimum_height(LayoutMode::Split, gap),
            config::SOURCES_HEIGHT_MIN
                + config::DATABASE_METRICS_HEIGHT_MIN
                + config::TRENDS_PANEL_HEIGHT_MIN
                + gap * 2.0,
        );
    }
}
