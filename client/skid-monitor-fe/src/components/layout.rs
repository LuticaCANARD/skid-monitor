use crate::config;
use eframe::egui::{self, Vec2};

#[derive(Clone, Copy)]
pub(crate) enum LayoutMode {
    Compact,
    Stacked,
    Split,
}

impl LayoutMode {
    pub(crate) fn for_width(width: f32) -> Self {
        if width < config::COMPACT_BREAKPOINT {
            Self::Compact
        } else if width < config::SPLIT_BREAKPOINT {
            Self::Stacked
        } else {
            Self::Split
        }
    }

    pub(crate) fn is_compact(self) -> bool {
        matches!(self, Self::Compact)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PanelLimits {
    pub(crate) main_height: f32,
    pub(crate) sources_height: f32,
    pub(crate) trends_height: f32,
    pub(crate) metrics_height: f32,
    pub(crate) event_log_height: f32,
}

impl PanelLimits {
    pub(crate) fn for_remaining_height(height: f32, layout: LayoutMode, section_gap: f32) -> Self {
        let available_height = height.max(0.0);
        let main_height = config::MAIN_AREA_HEIGHT
            .min((available_height - config::EVENT_LOG_HEIGHT_MIN - section_gap).max(0.0));
        let event_log_height = (available_height - main_height - section_gap).max(0.0);

        match layout {
            LayoutMode::Split => {
                let sources_height = clamped_extent(
                    main_height,
                    config::SOURCES_HEIGHT_RATIO,
                    config::SOURCES_HEIGHT_MIN,
                    config::SOURCES_HEIGHT_MAX,
                )
                .min(main_height);
                let trends_height = (main_height - sources_height - section_gap).max(0.0);

                Self {
                    main_height,
                    sources_height,
                    trends_height,
                    metrics_height: main_height,
                    event_log_height,
                }
            }
            LayoutMode::Stacked | LayoutMode::Compact => {
                let panel_budget = (main_height - section_gap * 2.0).max(0.0);
                let sources_height = clamped_extent(
                    panel_budget,
                    config::SOURCES_HEIGHT_RATIO,
                    config::SOURCES_HEIGHT_MIN,
                    config::SOURCES_HEIGHT_MAX,
                )
                .min(panel_budget);
                let metrics_height = clamped_extent(
                    panel_budget,
                    config::METRICS_TABLE_HEIGHT_RATIO,
                    config::METRICS_TABLE_HEIGHT_MIN,
                    config::METRICS_TABLE_HEIGHT_MAX,
                )
                .min((panel_budget - sources_height).max(0.0));
                let trends_height = (panel_budget - sources_height - metrics_height).max(0.0);

                Self {
                    main_height,
                    sources_height,
                    trends_height,
                    metrics_height,
                    event_log_height,
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ContentLayout {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) side_margin: f32,
    pub(crate) bottom_margin: f32,
}

impl ContentLayout {
    pub(crate) fn for_viewport(size: Vec2) -> Self {
        let readable_margin = clamped_extent(
            size.x,
            config::CONTENT_SIDE_MARGIN_RATIO,
            config::CONTENT_SIDE_MARGIN_MIN,
            config::CONTENT_SIDE_MARGIN_MAX,
        );
        let width = (size.x - readable_margin * 2.0)
            .clamp(1.0, config::CONTENT_MAX_WIDTH)
            .min(size.x.max(1.0));
        let side_margin = ((size.x - width) * 0.5).max(0.0);

        Self {
            width,
            height: size.y.max(1.0),
            side_margin,
            bottom_margin: clamped_extent(
                size.y,
                config::CONTENT_BOTTOM_MARGIN_RATIO,
                config::CONTENT_BOTTOM_MARGIN_MIN,
                config::CONTENT_BOTTOM_MARGIN_MAX,
            ),
        }
    }
}

pub(crate) fn remaining_height(ui: &egui::Ui, content: ContentLayout) -> f32 {
    let consumed_height = (ui.cursor().top() - ui.min_rect().top()).max(0.0);
    (content.height - consumed_height - content.bottom_margin).max(0.0)
}

pub(crate) fn section_gap(ui: &egui::Ui) -> f32 {
    config::SECTION_GAP + ui.spacing().item_spacing.y
}

pub(crate) fn panel_body_height(panel_height: f32) -> f32 {
    (panel_height - config::PANEL_HEADER_HEIGHT).max(1.0)
}

pub(crate) fn panel_frame<R>(
    ui: &mut egui::Ui,
    outer_width: f32,
    outer_height: f32,
    add_contents: impl FnOnce(&mut egui::Ui, Vec2) -> R,
) -> egui::InnerResponse<R> {
    let frame = egui::Frame::group(ui.style());
    let frame_margin = frame.total_margin().sum();
    let outer_width = outer_width.max(1.0);
    let inner_size = egui::vec2(
        (outer_width - frame_margin.x).max(1.0),
        (outer_height - frame_margin.y).max(1.0),
    );

    frame.show(ui, |ui| {
        ui.set_width(inner_size.x);
        ui.set_min_height(inner_size.y);
        add_contents(ui, inner_size)
    })
}

pub(crate) fn centered_content<R>(
    ui: &mut egui::Ui,
    content: ContentLayout,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let viewport = ui.clip_rect();
    let top = ui.cursor().top();
    let max_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.left() + content.side_margin, top),
        egui::vec2(content.width, content.height),
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("centered-content")
            .max_rect(max_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(content.width);
            add_contents(ui)
        },
    )
}

pub(crate) fn graph_panel_width(content_width: f32) -> f32 {
    (content_width * config::GRAPH_PANEL_WIDTH_RATIO)
        .clamp(config::GRAPH_PANEL_MIN_WIDTH, config::GRAPH_PANEL_MAX_WIDTH)
}

pub(crate) fn sparkline_height(width: f32) -> f32 {
    (width * config::SPARKLINE_HEIGHT_RATIO)
        .clamp(config::SPARKLINE_HEIGHT_MIN, config::SPARKLINE_HEIGHT_MAX)
}

fn clamped_extent(total: f32, ratio: f32, min: f32, max: f32) -> f32 {
    (total * ratio).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_section_gap() -> f32 {
        config::SECTION_GAP + config::GLOBAL_ITEM_SPACING.y
    }

    fn assert_close(left: f32, right: f32) {
        assert!(
            (left - right).abs() < 0.001,
            "expected {left} to match {right}"
        );
    }

    #[test]
    fn split_main_columns_end_at_the_same_height() {
        let gap = test_section_gap();
        let limits = PanelLimits::for_remaining_height(520.0, LayoutMode::Split, gap);

        assert_close(
            limits.sources_height + gap + limits.trends_height,
            limits.metrics_height,
        );
    }

    #[test]
    fn stacked_main_panels_include_actual_gaps_in_the_height_budget() {
        let gap = test_section_gap();
        let limits = PanelLimits::for_remaining_height(760.0, LayoutMode::Stacked, gap);

        assert_close(
            limits.sources_height + gap + limits.trends_height + gap + limits.metrics_height,
            config::MAIN_AREA_HEIGHT,
        );
    }
}
