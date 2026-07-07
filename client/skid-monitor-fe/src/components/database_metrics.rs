use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::table_header;
use crate::config;
use crate::model::MetricSample;
use crate::utils::shorten;
use eframe::egui::{self, RichText};

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        let database_metrics = metrics
            .iter()
            .copied()
            .filter(|sample| sample.is_database())
            .collect::<Vec<_>>();

        ui.heading("Database Metrics");
        ui.separator();
        if database_metrics.is_empty() {
            ui.label(
                RichText::new("no MySQL/PostgreSQL/Redis/Valkey metrics received yet")
                    .color(config::PLACEHOLDER_TEXT_COLOR),
            );
            return;
        }

        if compact {
            compact_database_table(
                ui,
                inner_size.x,
                panel_body_height(inner_size.y),
                &database_metrics,
            );
        } else {
            wide_database_table(
                ui,
                inner_size.x,
                panel_body_height(inner_size.y),
                &database_metrics,
            );
        }
    });
}

fn compact_database_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
) {
    let row_width = ui.available_width().min(panel_width).max(1.0);
    let spacing = ui.spacing().item_spacing.x;
    let system_width = config::DATABASE_METRICS_COMPACT_SYSTEM_WIDTH.min(row_width);
    let value_width = (row_width * 0.26).clamp(
        config::DATABASE_METRICS_COMPACT_VALUE_WIDTH_MIN,
        config::DATABASE_METRICS_COMPACT_VALUE_WIDTH_MAX,
    );
    let metric_width = (row_width - system_width - value_width - spacing * 2.0)
        .max(config::DATABASE_METRICS_COMPACT_METRIC_MIN_WIDTH);

    ui.horizontal(|ui| {
        ui.add_sized(
            [system_width, config::DATABASE_METRICS_ROW_HEIGHT],
            egui::Label::new(
                RichText::new("db")
                    .strong()
                    .color(config::TABLE_HEADER_COLOR),
            ),
        );
        ui.add_sized(
            [metric_width, config::DATABASE_METRICS_ROW_HEIGHT],
            egui::Label::new(
                RichText::new("metric")
                    .strong()
                    .color(config::TABLE_HEADER_COLOR),
            ),
        );
        ui.add_sized(
            [value_width, config::DATABASE_METRICS_ROW_HEIGHT],
            egui::Label::new(
                RichText::new("value")
                    .strong()
                    .color(config::TABLE_HEADER_COLOR),
            ),
        );
    });

    egui::ScrollArea::vertical()
        .id_salt("database-metrics-table-scroll-compact")
        .auto_shrink([false, false])
        .max_width(row_width)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_width(row_width);
            for sample in metrics.iter().rev().copied() {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [system_width, config::DATABASE_METRICS_ROW_HEIGHT],
                        egui::Label::new(RichText::new(sample.database_system_label()).monospace()),
                    );
                    ui.add_sized(
                        [metric_width, config::DATABASE_METRICS_ROW_HEIGHT],
                        egui::Label::new(RichText::new(shorten(&sample.name, 28)).monospace()),
                    );
                    ui.add_sized(
                        [value_width, config::DATABASE_METRICS_ROW_HEIGHT],
                        egui::Label::new(RichText::new(&sample.value).monospace().strong()),
                    );
                });
            }
        });
}

fn wide_database_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
) {
    let table_width = panel_width.max(config::DATABASE_METRICS_WIDE_SCROLL_MIN_WIDTH);

    egui::ScrollArea::both()
        .id_salt("database-metrics-table-scroll-wide")
        .auto_shrink([false, false])
        .max_width(panel_width)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(table_width);
            egui::Grid::new("database-metrics-grid-wide")
                .striped(true)
                .min_col_width(config::DATABASE_METRICS_WIDE_MIN_COL_WIDTH)
                .show(ui, |ui| {
                    table_header(ui, "db");
                    table_header(ui, "metric");
                    table_header(ui, "value");
                    table_header(ui, "namespace");
                    table_header(ui, "operation");
                    table_header(ui, "target");
                    table_header(ui, "node");
                    ui.end_row();

                    for sample in metrics.iter().rev().copied() {
                        ui.label(RichText::new(sample.database_system_label()).monospace());
                        ui.label(RichText::new(&sample.name).monospace());
                        ui.label(RichText::new(&sample.value).monospace().strong());
                        ui.label(RichText::new(&sample.database_namespace).monospace());
                        ui.label(RichText::new(&sample.database_operation).monospace());
                        ui.label(RichText::new(&sample.database_target).monospace());
                        ui.label(RichText::new(&sample.node).monospace());
                        ui.end_row();
                    }
                });
        });
}
