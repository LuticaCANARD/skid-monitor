use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::{alert_color, table_header};
use crate::config;
use crate::edge::{EdgeSignalDecoration, EdgeSignalDecorations, edge_key};
use crate::model::{AlertSeverity, NodeSummary};
use crate::utils::{format_duration, shorten};
use eframe::egui::{self, Color32, RichText, Stroke};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct AddAgentDraft {
    pub(crate) endpoint: String,
    pub(crate) node: String,
    pub(crate) service: String,
}

impl AddAgentDraft {
    pub(crate) fn clear(&mut self) {
        self.endpoint.clear();
        self.node.clear();
        self.service.clear();
    }
}

pub(crate) struct AgentNotice<'a> {
    pub(crate) message: &'a str,
    pub(crate) is_error: bool,
}

pub(crate) enum AgentOverviewAction {
    Select(String),
    StartAdd,
    CancelAdd,
    SaveAdd {
        endpoint: String,
        node: String,
        service: String,
    },
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    nodes: &BTreeMap<String, NodeSummary>,
    decorations: &EdgeSignalDecorations,
    draft: &mut AddAgentDraft,
    show_form: bool,
    notice: Option<AgentNotice<'_>>,
    panel_width: f32,
    max_height: f32,
) -> Option<AgentOverviewAction> {
    let mut action = None;

    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        let panel_top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.heading("Agents");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("+ Add agent").clicked() {
                    action = Some(AgentOverviewAction::StartAdd);
                }
            });
        });
        ui.separator();

        if let Some(notice) = notice {
            let color = if notice.is_error {
                config::STATUS_ERROR_COLOR
            } else {
                config::STATUS_LISTENING_COLOR
            };
            ui.label(RichText::new(notice.message).color(color));
            ui.add_space(config::SECTION_GAP * 0.5);
        }

        if show_form {
            if let Some(next_action) = add_agent_form(ui, compact, draft, inner_size.x) {
                action = Some(next_action);
            }
            ui.add_space(config::SECTION_GAP);
        }

        if nodes.is_empty() {
            ui.label(RichText::new("no agents yet").color(config::PLACEHOLDER_TEXT_COLOR));
            return;
        }

        let consumed_height = (ui.cursor().top() - panel_top).max(0.0);
        let table_height = (inner_size.y - consumed_height).max(80.0);
        agent_table(
            ui,
            compact,
            nodes,
            decorations,
            inner_size.x,
            panel_body_height(table_height),
            &mut action,
        );
    });

    action
}

fn add_agent_form(
    ui: &mut egui::Ui,
    compact: bool,
    draft: &mut AddAgentDraft,
    width: f32,
) -> Option<AgentOverviewAction> {
    let mut action = None;
    let form_width = width.max(1.0);
    let frame = egui::Frame::default()
        .fill(config::STAT_TILE_BACKGROUND)
        .stroke(Stroke::new(
            config::STAT_TILE_BORDER_WIDTH,
            config::STAT_TILE_BORDER,
        ))
        .corner_radius(egui::CornerRadius::same(config::STAT_TILE_RADIUS))
        .inner_margin(egui::Margin::same(config::STAT_TILE_MARGIN));

    frame.show(ui, |ui| {
        ui.set_width((form_width - f32::from(config::STAT_TILE_MARGIN) * 2.0).max(1.0));
        if compact {
            form_field(ui, "endpoint", &mut draft.endpoint);
            form_field(ui, "node", &mut draft.node);
            form_field(ui, "service", &mut draft.service);
        } else {
            ui.horizontal(|ui| {
                form_field(ui, "endpoint", &mut draft.endpoint);
                form_field(ui, "node", &mut draft.node);
                form_field(ui, "service", &mut draft.service);
            });
        }
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                action = Some(AgentOverviewAction::SaveAdd {
                    endpoint: draft.endpoint.clone(),
                    node: draft.node.clone(),
                    service: draft.service.clone(),
                });
            }
            if ui.button("Cancel").clicked() {
                action = Some(AgentOverviewAction::CancelAdd);
            }
        });
    });

    action
}

fn form_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).color(config::TABLE_HEADER_COLOR));
        ui.add(egui::TextEdit::singleline(value).desired_width(190.0));
    });
}

fn agent_table(
    ui: &mut egui::Ui,
    compact: bool,
    nodes: &BTreeMap<String, NodeSummary>,
    decorations: &EdgeSignalDecorations,
    panel_width: f32,
    max_height: f32,
    action: &mut Option<AgentOverviewAction>,
) {
    let table_width = if compact {
        panel_width.max(680.0)
    } else {
        panel_width.max(1040.0)
    };
    let rows = recent_rows(nodes);

    egui::ScrollArea::both()
        .id_salt("agents-table-scroll")
        .auto_shrink([false, false])
        .max_width(panel_width)
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(table_width);
            egui::Grid::new("agents-grid")
                .striped(true)
                .min_col_width(if compact { 56.0 } else { 72.0 })
                .show(ui, |ui| {
                    table_header(ui, "agent");
                    table_header(ui, "state");
                    table_header(ui, "endpoint");
                    table_header(ui, "source");
                    table_header(ui, "service");
                    table_header(ui, "signals");
                    table_header(ui, "last");
                    table_header(ui, "age");
                    table_header(ui, "");
                    ui.end_row();

                    for (key, row) in rows {
                        let decoration = decorations.get(&row.endpoint, &row.node);
                        let (state_label, state_color) = state_label(row, decoration);
                        ui.label(RichText::new(shorten(&row.node, 36)).monospace());
                        ui.label(RichText::new(state_label).monospace().color(state_color));
                        ui.label(RichText::new(shorten(&row.endpoint, 32)).monospace());
                        ui.label(RichText::new(shorten(&row.source, 24)).monospace());
                        ui.label(RichText::new(shorten(&row.service, 24)).monospace());
                        ui.label(RichText::new(signal_count(row).to_string()).strong());
                        ui.label(RichText::new(shorten(&last_signal(row), 34)).monospace());
                        ui.label(RichText::new(age(row)).monospace());
                        if ui.button("Open").clicked() {
                            *action = Some(AgentOverviewAction::Select(key));
                        }
                        ui.end_row();
                    }
                });
        });
}

fn recent_rows(nodes: &BTreeMap<String, NodeSummary>) -> Vec<(String, &NodeSummary)> {
    let mut rows = nodes
        .values()
        .map(|row| (edge_key(&row.endpoint, &row.node), row))
        .collect::<Vec<_>>();
    rows.sort_by(|(_, left), (_, right)| {
        right
            .last_seen
            .cmp(&left.last_seen)
            .then_with(|| left.node.cmp(&right.node))
    });
    rows
}

fn state_label(
    row: &NodeSummary,
    decoration: Option<&EdgeSignalDecoration>,
) -> (&'static str, Color32) {
    match decoration.and_then(|edge| edge.severity) {
        Some(AlertSeverity::Critical) => ("critical", alert_color(AlertSeverity::Critical)),
        Some(AlertSeverity::Warning) => ("warning", alert_color(AlertSeverity::Warning)),
        None if signal_count(row) == 0 => ("pending", config::MUTED_TEXT_COLOR),
        None => ("online", config::STATUS_LISTENING_COLOR),
    }
}

fn signal_count(row: &NodeSummary) -> usize {
    row.metric_points + row.spans + row.log_records
}

fn last_signal(row: &NodeSummary) -> String {
    if row.last_metric.is_empty() {
        row.last_value.clone()
    } else if row.last_value.is_empty() {
        row.last_metric.clone()
    } else {
        format!("{} {}", row.last_metric, row.last_value)
    }
}

fn age(row: &NodeSummary) -> String {
    format_duration(Instant::now().saturating_duration_since(row.last_seen))
}
