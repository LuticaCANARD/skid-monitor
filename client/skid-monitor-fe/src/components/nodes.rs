use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::{alert_color, table_header};
use crate::config;
use crate::edge::{EdgeSignalDecoration, EdgeSignalDecorations};
use crate::model::AlertSeverity;
use crate::model::NodeSummary;
use crate::utils::{format_duration, shorten};
use eframe::egui::{self, Color32, RichText};
use std::time::Instant;

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    nodes: &[&NodeSummary],
    decorations: &EdgeSignalDecorations,
    panel_width: f32,
    max_height: f32,
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        ui.set_min_width(config::NODE_TABLE_MIN_WIDTH.min(inner_size.x));
        ui.heading("Nodes");
        ui.separator();
        if nodes.is_empty() {
            ui.label(
                RichText::new("waiting for node signals").color(config::PLACEHOLDER_TEXT_COLOR),
            );
            return;
        }

        if compact {
            compact_node_table(
                ui,
                inner_size.x,
                panel_body_height(inner_size.y),
                nodes,
                decorations,
            );
        } else {
            wide_node_table(
                ui,
                inner_size.x,
                panel_body_height(inner_size.y),
                nodes,
                decorations,
            );
        }
    });
}

fn compact_node_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    nodes: &[&NodeSummary],
    decorations: &EdgeSignalDecorations,
) {
    let row_width = panel_width.max(1.0);
    let marker_width = config::NODE_EDGE_MARKER_WIDTH;
    let spacing = ui.spacing().item_spacing.x;
    let usable_width = (row_width - marker_width - spacing * 3.0).max(1.0);
    let node_width = (usable_width * 0.42).clamp(82.0, 220.0);
    let signals_width = (usable_width * 0.22).clamp(52.0, 110.0);
    let last_width = (usable_width - node_width - signals_width).max(72.0);
    let now = Instant::now();
    let rows = recent_rows(nodes);

    ui.horizontal(|ui| {
        ui.add_sized([marker_width, 20.0], egui::Label::new(""));
        ui.add_sized([node_width, 20.0], egui::Label::new(header_text("node")));
        ui.add_sized(
            [signals_width, 20.0],
            egui::Label::new(header_text("signals")),
        );
        ui.add_sized([last_width, 20.0], egui::Label::new(header_text("last")));
    });

    egui::ScrollArea::vertical()
        .id_salt("nodes-table-scroll-compact")
        .auto_shrink([false, false])
        .max_width(row_width)
        .max_height(max_height)
        .show(ui, |ui| {
            for row in rows {
                let decoration = decorations.get(&row.endpoint, &row.node);
                ui.horizontal(|ui| {
                    paint_edge_marker(ui, decoration);
                    ui.add_sized(
                        [node_width, 22.0],
                        egui::Label::new(
                            RichText::new(shorten(&row.node, 24))
                                .monospace()
                                .color(edge_color(decoration)),
                        ),
                    );
                    ui.add_sized(
                        [signals_width, 22.0],
                        egui::Label::new(RichText::new(signal_count(row).to_string()).strong()),
                    );
                    ui.add_sized(
                        [last_width, 22.0],
                        egui::Label::new(
                            RichText::new(shorten(&last_cell(row, now), 28)).monospace(),
                        ),
                    );
                });
            }
        });
}

fn wide_node_table(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    nodes: &[&NodeSummary],
    decorations: &EdgeSignalDecorations,
) {
    let table_width = panel_width.max(980.0);
    let now = Instant::now();
    let rows = recent_rows(nodes);

    egui::ScrollArea::both()
        .id_salt("nodes-table-scroll-wide")
        .auto_shrink([false, false])
        .max_width(panel_width)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(table_width);
            egui::Grid::new("nodes-grid-wide")
                .striped(true)
                .min_col_width(72.0)
                .show(ui, |ui| {
                    table_header(ui, "node");
                    table_header(ui, "state");
                    table_header(ui, "ingress");
                    table_header(ui, "source");
                    table_header(ui, "service");
                    table_header(ui, "points");
                    table_header(ui, "spans");
                    table_header(ui, "logs");
                    table_header(ui, "last");
                    table_header(ui, "value");
                    table_header(ui, "age");
                    ui.end_row();

                    for row in rows {
                        let decoration = decorations.get(&row.endpoint, &row.node);
                        ui.label(RichText::new(&row.node).monospace());
                        ui.label(edge_label(decoration));
                        ui.label(RichText::new(&row.endpoint).monospace());
                        ui.label(RichText::new(&row.source).monospace());
                        ui.label(RichText::new(&row.service).monospace());
                        ui.label(RichText::new(row.metric_points.to_string()).strong());
                        ui.label(row.spans.to_string());
                        ui.label(row.log_records.to_string());
                        ui.label(RichText::new(&row.last_metric).monospace());
                        ui.label(RichText::new(&row.last_value).monospace().strong());
                        ui.label(RichText::new(age(row, now)).monospace());
                        ui.end_row();
                    }
                });
        });
}

fn header_text(label: &str) -> RichText {
    RichText::new(label)
        .strong()
        .color(config::TABLE_HEADER_COLOR)
}

fn recent_rows<'a>(nodes: &[&'a NodeSummary]) -> Vec<&'a NodeSummary> {
    let mut rows = nodes.to_vec();
    rows.sort_by(|left, right| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| left.node.cmp(&right.node))
    });
    rows
}

fn signal_count(row: &NodeSummary) -> usize {
    row.metric_points + row.spans + row.log_records
}

fn last_cell(row: &NodeSummary, now: Instant) -> String {
    format!("{} {} {}", row.last_metric, row.last_value, age(row, now))
}

fn age(row: &NodeSummary, now: Instant) -> String {
    format_duration(now.saturating_duration_since(row.last_seen))
}

fn paint_edge_marker(ui: &mut egui::Ui, decoration: Option<&EdgeSignalDecoration>) {
    let desired = egui::vec2(
        config::NODE_EDGE_MARKER_WIDTH,
        config::NODE_EDGE_MARKER_HEIGHT,
    );
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(2), edge_color(decoration));
}

fn edge_label(decoration: Option<&EdgeSignalDecoration>) -> RichText {
    let label = match decoration.and_then(|edge| edge.severity) {
        Some(AlertSeverity::Critical) => "critical".to_string(),
        Some(AlertSeverity::Warning) => "warning".to_string(),
        None => decoration
            .map(|edge| shorten(&edge.last_signal, 14))
            .unwrap_or_else(|| "clear".to_string()),
    };

    RichText::new(label)
        .monospace()
        .color(edge_color(decoration))
}

fn edge_color(decoration: Option<&EdgeSignalDecoration>) -> Color32 {
    match decoration.and_then(|edge| edge.severity) {
        Some(severity) => alert_color(severity),
        None if decoration.is_some() => config::STATUS_LISTENING_COLOR,
        None => config::MUTED_TEXT_COLOR,
    }
}
