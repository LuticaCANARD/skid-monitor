use crate::config;
use crate::model::{AlertSeverity, AlertSummary, Status};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::collections::VecDeque;

pub(crate) fn draw_sparkline(ui: &mut egui::Ui, values: &VecDeque<f64>, height: f32) {
    let desired = Vec2::new(ui.available_width().max(1.0), height);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(
        rect,
        egui::CornerRadius::same(config::SPARKLINE_RADIUS),
        config::SPARKLINE_BACKGROUND,
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(config::SPARKLINE_RADIUS),
        Stroke::new(config::SPARKLINE_BORDER_WIDTH, config::SPARKLINE_BORDER),
        egui::StrokeKind::Inside,
    );

    if values.len() < 2 {
        return;
    }

    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for value in values {
        if value.is_finite() {
            min = min.min(*value);
            max = max.max(*value);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return;
    }
    if (max - min).abs() < f64::EPSILON {
        max += 1.0;
        min -= 1.0;
    }

    let left = rect.left() + config::SPARKLINE_PADDING_X;
    let right = rect.right() - config::SPARKLINE_PADDING_X;
    let top = rect.top() + config::SPARKLINE_PADDING_Y;
    let bottom = rect.bottom() - config::SPARKLINE_PADDING_Y;
    let width = (right - left).max(1.0);
    let height = (bottom - top).max(1.0);
    let last_index = (values.len() - 1) as f32;

    let mut points = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let x = left + width * (index as f32 / last_index);
        let normalized = ((*value - min) / (max - min)).clamp(0.0, 1.0) as f32;
        let y = bottom - height * normalized;
        points.push(egui::pos2(x, y));
    }

    for pair in points.windows(2) {
        painter.line_segment(
            [pair[0], pair[1]],
            Stroke::new(config::SPARKLINE_STROKE_WIDTH, config::SPARKLINE_LINE),
        );
    }
}

pub(crate) fn status_badge(ui: &mut egui::Ui, status: &Status) {
    let (label, color) = status_badge_label(status);

    egui::Frame::default()
        .fill(config::STATUS_BADGE_BACKGROUND)
        .stroke(Stroke::new(config::STATUS_BADGE_BORDER_WIDTH, color))
        .corner_radius(egui::CornerRadius::same(config::STATUS_BADGE_RADIUS))
        .inner_margin(egui::Margin::symmetric(
            config::STATUS_BADGE_MARGIN_X,
            config::STATUS_BADGE_MARGIN_Y,
        ))
        .show(ui, |ui| {
            ui.label(RichText::new(label).monospace().color(color));
        });
}

pub(crate) fn status_badge_width(ui: &egui::Ui, status: &Status) -> f32 {
    let (label, color) = status_badge_label(status);
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let text_width = ui.painter().layout_no_wrap(label, font_id, color).size().x;

    text_width + f32::from(config::STATUS_BADGE_MARGIN_X) * 2.0
}

pub(crate) fn alert_badge(ui: &mut egui::Ui, summary: AlertSummary) {
    let (label, color) = alert_badge_label(summary);

    egui::Frame::default()
        .fill(config::ALERT_BADGE_BACKGROUND)
        .stroke(Stroke::new(config::ALERT_BADGE_BORDER_WIDTH, color))
        .corner_radius(egui::CornerRadius::same(config::ALERT_BADGE_RADIUS))
        .inner_margin(egui::Margin::symmetric(
            config::ALERT_BADGE_MARGIN_X,
            config::ALERT_BADGE_MARGIN_Y,
        ))
        .show(ui, |ui| {
            ui.label(RichText::new(label).monospace().color(color));
        });
}

pub(crate) fn alert_badge_width(ui: &egui::Ui, summary: AlertSummary) -> f32 {
    let (label, color) = alert_badge_label(summary);
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let text_width = ui.painter().layout_no_wrap(label, font_id, color).size().x;

    text_width + f32::from(config::ALERT_BADGE_MARGIN_X) * 2.0
}

pub(crate) fn alert_color(severity: AlertSeverity) -> Color32 {
    match severity {
        AlertSeverity::Warning => config::ALERT_WARNING_COLOR,
        AlertSeverity::Critical => config::ALERT_CRITICAL_COLOR,
    }
}

fn status_badge_label(status: &Status) -> (String, Color32) {
    match status {
        Status::Starting => ("starting".to_string(), config::STATUS_STARTING_COLOR),
        Status::Listening(addr) => (format!("listening {addr}"), config::STATUS_LISTENING_COLOR),
        Status::Error(error) => (error.clone(), config::STATUS_ERROR_COLOR),
    }
}

fn alert_badge_label(summary: AlertSummary) -> (String, Color32) {
    match summary.highest_severity {
        Some(AlertSeverity::Critical) => (
            format!("critical x{}", summary.active_count),
            config::ALERT_CRITICAL_COLOR,
        ),
        Some(AlertSeverity::Warning) => (
            format!("warning x{}", summary.active_count),
            config::ALERT_WARNING_COLOR,
        ),
        None => ("alerts clear".to_string(), config::ALERT_CLEAR_COLOR),
    }
}

pub(crate) fn stat_tile(ui: &mut egui::Ui, label: &str, value: usize, width: f32) {
    let inner_width = (width - f32::from(config::STAT_TILE_MARGIN) * 2.0).max(1.0);
    let value_text = value.to_string();
    let number_width = (value_text.chars().count() as f32 * config::STAT_TILE_NUMBER_CHAR_WIDTH)
        .clamp(
            config::STAT_TILE_NUMBER_MIN_WIDTH,
            config::STAT_TILE_NUMBER_MAX_WIDTH
                .min(inner_width * config::STAT_TILE_NUMBER_MAX_RATIO),
        );
    let label_width = (inner_width - number_width - config::STAT_TILE_LABEL_GAP).max(1.0);

    egui::Frame::default()
        .fill(config::STAT_TILE_BACKGROUND)
        .stroke(Stroke::new(
            config::STAT_TILE_BORDER_WIDTH,
            config::STAT_TILE_BORDER,
        ))
        .corner_radius(egui::CornerRadius::same(config::STAT_TILE_RADIUS))
        .inner_margin(egui::Margin::same(config::STAT_TILE_MARGIN))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(inner_width, config::STAT_TILE_CONTENT_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = config::STAT_TILE_LABEL_GAP;
                    ui.add_sized(
                        [number_width, config::STAT_TILE_CONTENT_HEIGHT],
                        egui::Label::new(
                            RichText::new(value_text)
                                .size(config::STAT_TILE_NUMBER_SIZE)
                                .strong(),
                        ),
                    );
                    ui.add_sized(
                        [label_width, config::STAT_TILE_CONTENT_HEIGHT],
                        egui::Label::new(
                            RichText::new(label)
                                .size(config::STAT_TILE_LABEL_SIZE)
                                .color(config::STAT_TILE_LABEL_COLOR),
                        )
                        .truncate(),
                    );
                },
            );
        });
}

pub(crate) fn table_header(ui: &mut egui::Ui, label: &str) {
    ui.label(
        RichText::new(label)
            .strong()
            .color(config::TABLE_HEADER_COLOR),
    );
}

pub(crate) fn kind_color(kind: &str) -> Color32 {
    match kind {
        "metrics" => config::EVENT_METRICS_COLOR,
        "traces" => config::EVENT_TRACES_COLOR,
        "logs" => config::EVENT_LOGS_COLOR,
        "error" => config::EVENT_ERROR_COLOR,
        "extension" => config::EVENT_EXTENSION_COLOR,
        "alert" => config::EVENT_ALERT_COLOR,
        "resolved" => config::EVENT_RESOLVED_COLOR,
        _ => config::EVENT_DEFAULT_COLOR,
    }
}
