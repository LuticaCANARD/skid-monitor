use crate::alert::AlertStore;
use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::{alert_color, table_header};
use crate::config;
use crate::model::{AlertSeverity, MetricSample};
use crate::utils::shorten;
use eframe::egui::{self, RichText, Stroke};

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
    alerts: &AlertStore,
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        let panel_width = inner_size.x;
        ui.heading("Latest Metrics");
        ui.separator();
        if metrics.is_empty() {
            ui.label(
                RichText::new("no metrics received yet").color(config::PLACEHOLDER_TEXT_COLOR),
            );
            return;
        }

        if compact {
            compact_metrics_table(
                ui,
                panel_width,
                panel_body_height(inner_size.y),
                metrics,
                alerts,
            );
        } else {
            wide_metrics_table(
                ui,
                panel_width,
                panel_body_height(inner_size.y),
                metrics,
                alerts,
            );
        }
    });
}

fn compact_metrics_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
    alerts: &AlertStore,
) {
    let row_width = ui.available_width().min(panel_width).max(1.0);
    let spacing = ui.spacing().item_spacing.x;
    let value_width = (row_width * config::METRICS_COMPACT_VALUE_WIDTH_RATIO).clamp(
        config::METRICS_COMPACT_VALUE_WIDTH_MIN,
        config::METRICS_COMPACT_VALUE_WIDTH_MAX,
    );
    let name_width =
        (row_width - value_width - spacing).max(config::METRICS_COMPACT_NAME_MIN_WIDTH);
    let name_chars = ((name_width / config::METRICS_COMPACT_NAME_CHAR_WIDTH).floor() as usize)
        .clamp(
            config::METRICS_COMPACT_NAME_CHARS_MIN,
            config::METRICS_COMPACT_NAME_CHARS_MAX,
        );
    let row_area_height = (max_height - config::METRICS_COMPACT_ROW_HEIGHT)
        .max(config::METRICS_COMPACT_ROW_AREA_MIN_HEIGHT);

    ui.horizontal(|ui| {
        ui.add_sized(
            [name_width, config::METRICS_COMPACT_ROW_HEIGHT],
            egui::Label::new(
                RichText::new("metric")
                    .strong()
                    .color(config::TABLE_HEADER_COLOR),
            ),
        );
        ui.add_sized(
            [value_width, config::METRICS_COMPACT_ROW_HEIGHT],
            egui::Label::new(
                RichText::new("value")
                    .strong()
                    .color(config::TABLE_HEADER_COLOR),
            ),
        );
    });

    egui::ScrollArea::vertical()
        .id_salt("metrics-table-scroll-compact")
        .auto_shrink([false, false])
        .max_width(row_width)
        .max_height(row_area_height)
        .show(ui, |ui| {
            ui.set_width(row_width);
            for (index, sample) in metrics.iter().rev().copied().enumerate() {
                let severity = alerts.active_for_metric(sample);
                let fill = row_fill(index, severity);
                let stroke = severity
                    .map(|severity| Stroke::new(1.0, alert_color(severity)))
                    .unwrap_or(Stroke::NONE);

                egui::Frame::default()
                    .fill(fill)
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [name_width, config::METRICS_COMPACT_ROW_HEIGHT],
                                egui::Label::new(
                                    RichText::new(shorten(&sample.name, name_chars)).monospace(),
                                ),
                            );
                            ui.add_sized(
                                [value_width, config::METRICS_COMPACT_ROW_HEIGHT],
                                egui::Label::new(RichText::new(&sample.value).monospace().strong()),
                            );
                        });
                    });
            }
        });
}

fn wide_metrics_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
    alerts: &AlertStore,
) {
    let table_width = panel_width.max(config::METRICS_WIDE_SCROLL_MIN_WIDTH);

    egui::ScrollArea::both()
        .id_salt("metrics-table-scroll-wide")
        .auto_shrink([false, false])
        .max_width(panel_width)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(table_width);
            egui::Grid::new("metrics-grid-wide")
                .striped(true)
                .min_col_width(config::METRICS_WIDE_MIN_COL_WIDTH)
                .show(ui, |ui| {
                    table_header(ui, "metric");
                    table_header(ui, "value");
                    table_header(ui, "node");
                    table_header(ui, "source");
                    table_header(ui, "kind");
                    table_header(ui, "attrs");
                    ui.end_row();

                    for sample in metrics.iter().rev().copied() {
                        let severity = alerts.active_for_metric(sample);
                        let metric = match severity {
                            Some(severity) => RichText::new(&sample.name)
                                .monospace()
                                .color(alert_color(severity))
                                .strong(),
                            None => RichText::new(&sample.name).monospace(),
                        };
                        let value = match severity {
                            Some(severity) => RichText::new(&sample.value)
                                .monospace()
                                .strong()
                                .color(alert_color(severity)),
                            None => RichText::new(&sample.value).monospace().strong(),
                        };

                        ui.label(metric);
                        ui.label(value);
                        ui.label(RichText::new(&sample.node).monospace());
                        ui.label(RichText::new(&sample.source).monospace());
                        ui.label(&sample.kind);
                        ui.label(RichText::new(&sample.attributes).small());
                        ui.end_row();
                    }
                });
        });
}

fn row_fill(index: usize, severity: Option<AlertSeverity>) -> egui::Color32 {
    match severity {
        Some(AlertSeverity::Critical) => config::ALERT_ROW_CRITICAL,
        Some(AlertSeverity::Warning) => config::ALERT_ROW_WARNING,
        None if index % 2 == 0 => config::METRICS_COMPACT_ROW_EVEN,
        None => config::METRICS_COMPACT_ROW_ODD,
    }
}
