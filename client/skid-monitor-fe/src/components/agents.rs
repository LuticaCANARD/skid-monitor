use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::{alert_color, table_header};
use crate::config;
use crate::edge::{EdgeSignalDecoration, EdgeSignalDecorations, edge_key};
use crate::model::{AlertSeverity, NodeSummary};
use crate::utils::{format_duration, shorten};
use eframe::egui::{self, Color32, RichText, Stroke};
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Default)]
pub(crate) struct ListenerDraft {
    pub(crate) addr: String,
}

impl ListenerDraft {
    pub(crate) fn clear(&mut self) {
        self.addr.clear();
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
    RequestRemove(String),
    ConfirmRemove(String),
    CancelRemove,
    SaveListener(String),
    RequestRemoveListener(String),
    ConfirmRemoveListener(String),
    CancelRemoveListener,
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    nodes: &BTreeMap<String, NodeSummary>,
    listeners: &BTreeSet<String>,
    decorations: &EdgeSignalDecorations,
    draft: &mut AddAgentDraft,
    listener_draft: &mut ListenerDraft,
    filter: &mut String,
    show_form: bool,
    pending_remove_key: Option<&str>,
    pending_remove_listener: Option<&str>,
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
        listener_bar(
            ui,
            compact,
            listeners,
            listener_draft,
            pending_remove_listener,
            &mut action,
        );
        ui.add_space(config::SECTION_GAP * 0.5);
        filter_bar(ui, compact, filter);
        ui.add_space(config::SECTION_GAP * 0.5);

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

        let rows = recent_rows(nodes, filter);
        if nodes.is_empty() {
            ui.label(RichText::new("no agents yet").color(config::PLACEHOLDER_TEXT_COLOR));
            return;
        }
        if rows.is_empty() {
            ui.label(RichText::new("no agents match filter").color(config::PLACEHOLDER_TEXT_COLOR));
            return;
        }

        let consumed_height = (ui.cursor().top() - panel_top).max(0.0);
        let table_height = (inner_size.y - consumed_height).max(80.0);
        agent_table(
            ui,
            compact,
            &rows,
            decorations,
            pending_remove_key,
            inner_size.x,
            panel_body_height(table_height),
            &mut action,
        );
    });

    action
}

fn listener_bar(
    ui: &mut egui::Ui,
    compact: bool,
    listeners: &BTreeSet<String>,
    draft: &mut ListenerDraft,
    pending_remove_listener: Option<&str>,
    action: &mut Option<AgentOverviewAction>,
) {
    let input_width = if compact {
        config::AGENT_FILTER_WIDTH_COMPACT
    } else {
        config::AGENT_FILTER_WIDTH_WIDE
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Ingress listeners").color(config::TABLE_HEADER_COLOR));
        ui.add(
            egui::TextEdit::singleline(&mut draft.addr)
                .desired_width(input_width)
                .hint_text("127.0.0.1:9000"),
        );
        let can_add = !draft.addr.trim().is_empty();
        let add = ui.add_enabled(can_add, egui::Button::new("Bind"));
        if add.clicked() {
            *action = Some(AgentOverviewAction::SaveListener(draft.addr.clone()));
        }
        if !can_add {
            add.on_hover_text("listen address is required");
        }
    });

    ui.horizontal_wrapped(|ui| {
        if listeners.is_empty() {
            ui.label(
                RichText::new("no active ingress listeners").color(config::PLACEHOLDER_TEXT_COLOR),
            );
            return;
        }

        for listener in listeners {
            listener_chip(ui, listener, pending_remove_listener, action);
        }
    });
}

fn listener_chip(
    ui: &mut egui::Ui,
    listener: &str,
    pending_remove_listener: Option<&str>,
    action: &mut Option<AgentOverviewAction>,
) {
    let remove_width = config::AGENT_ACTION_BUTTON_WIDTH;
    let button_height = ui.spacing().interact_size.y;

    ui.horizontal(|ui| {
        ui.label(RichText::new(listener).monospace());
        if pending_remove_listener == Some(listener) {
            if ui
                .add_sized(
                    [config::AGENT_CONFIRM_BUTTON_WIDTH, button_height],
                    egui::Button::new(RichText::new("Confirm").color(config::STATUS_ERROR_COLOR)),
                )
                .clicked()
            {
                *action = Some(AgentOverviewAction::ConfirmRemoveListener(
                    listener.to_string(),
                ));
            }
            if ui
                .add_sized([remove_width, button_height], egui::Button::new("Cancel"))
                .clicked()
            {
                *action = Some(AgentOverviewAction::CancelRemoveListener);
            }
        } else if ui
            .add_sized(
                [remove_width, button_height],
                egui::Button::new(RichText::new("Unbind").color(config::STATUS_ERROR_COLOR)),
            )
            .clicked()
        {
            *action = Some(AgentOverviewAction::RequestRemoveListener(
                listener.to_string(),
            ));
        }
    });
}

fn filter_bar(ui: &mut egui::Ui, compact: bool, filter: &mut String) {
    let filter_width = if compact {
        config::AGENT_FILTER_WIDTH_COMPACT
    } else {
        config::AGENT_FILTER_WIDTH_WIDE
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Filter").color(config::TABLE_HEADER_COLOR));
        ui.add(
            egui::TextEdit::singleline(filter)
                .desired_width(filter_width)
                .hint_text("node, ingress, source"),
        );
        if !filter.trim().is_empty() && ui.button("Clear").clicked() {
            filter.clear();
        }
    });
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
            form_field(
                ui,
                "ingress",
                &mut draft.endpoint,
                config::AGENT_FORM_FIELD_WIDTH_COMPACT,
                "127.0.0.1:9000",
            );
            form_field(
                ui,
                "node",
                &mut draft.node,
                config::AGENT_FORM_FIELD_WIDTH_COMPACT,
                "edge-a",
            );
            form_field(
                ui,
                "service",
                &mut draft.service,
                config::AGENT_FORM_FIELD_WIDTH_COMPACT,
                "skid-monitor-agent",
            );
        } else {
            ui.horizontal(|ui| {
                form_field(
                    ui,
                    "ingress",
                    &mut draft.endpoint,
                    config::AGENT_FORM_FIELD_WIDTH_WIDE,
                    "127.0.0.1:9000",
                );
                form_field(
                    ui,
                    "node",
                    &mut draft.node,
                    config::AGENT_FORM_FIELD_WIDTH_WIDE,
                    "edge-a",
                );
                form_field(
                    ui,
                    "service",
                    &mut draft.service,
                    config::AGENT_FORM_FIELD_WIDTH_WIDE,
                    "skid-monitor-agent",
                );
            });
        }
        ui.horizontal(|ui| {
            let can_save = !draft.endpoint.trim().is_empty();
            let save = ui.add_enabled(can_save, egui::Button::new("Save"));
            if save.clicked() {
                action = Some(AgentOverviewAction::SaveAdd {
                    endpoint: draft.endpoint.clone(),
                    node: draft.node.clone(),
                    service: draft.service.clone(),
                });
            }
            if !can_save {
                save.on_hover_text("ingress is required");
            }
            if ui.button("Cancel").clicked() {
                action = Some(AgentOverviewAction::CancelAdd);
            }
        });
    });

    action
}

fn form_field(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32, hint: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).color(config::TABLE_HEADER_COLOR));
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(width)
                .hint_text(hint),
        );
    });
}

fn agent_table(
    ui: &mut egui::Ui,
    compact: bool,
    rows: &[(String, &NodeSummary)],
    decorations: &EdgeSignalDecorations,
    pending_remove_key: Option<&str>,
    panel_width: f32,
    max_height: f32,
    action: &mut Option<AgentOverviewAction>,
) {
    let table_width = if compact {
        panel_width.max(680.0)
    } else {
        panel_width.max(1040.0)
    };

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
                .min_row_height(ui.spacing().interact_size.y)
                .show(ui, |ui| {
                    table_header(ui, "agent");
                    table_header(ui, "state");
                    table_header(ui, "ingress");
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
                        row_actions(ui, key, pending_remove_key, action);
                        ui.end_row();
                    }
                });
        });
}

fn row_actions(
    ui: &mut egui::Ui,
    key: &str,
    pending_remove_key: Option<&str>,
    action: &mut Option<AgentOverviewAction>,
) {
    let button_height = ui.spacing().interact_size.y;
    let action_width = config::AGENT_CONFIRM_BUTTON_WIDTH
        + config::AGENT_ACTION_BUTTON_WIDTH
        + ui.spacing().item_spacing.x;

    ui.allocate_ui_with_layout(
        egui::Vec2::new(action_width, button_height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if pending_remove_key == Some(key) {
                let confirm = egui::Button::new(
                    RichText::new("Confirm")
                        .strong()
                        .color(config::STATUS_ERROR_COLOR),
                );
                if ui
                    .add_sized([config::AGENT_CONFIRM_BUTTON_WIDTH, button_height], confirm)
                    .clicked()
                {
                    *action = Some(AgentOverviewAction::ConfirmRemove(key.to_string()));
                }
                if ui
                    .add_sized(
                        [config::AGENT_ACTION_BUTTON_WIDTH, button_height],
                        egui::Button::new("Cancel"),
                    )
                    .clicked()
                {
                    *action = Some(AgentOverviewAction::CancelRemove);
                }
            } else {
                if ui
                    .add_sized(
                        [config::AGENT_ACTION_BUTTON_WIDTH, button_height],
                        egui::Button::new("Open"),
                    )
                    .clicked()
                {
                    *action = Some(AgentOverviewAction::Select(key.to_string()));
                }
                if ui
                    .add_sized(
                        [config::AGENT_ACTION_BUTTON_WIDTH, button_height],
                        egui::Button::new(
                            RichText::new("Remove").color(config::STATUS_ERROR_COLOR),
                        ),
                    )
                    .clicked()
                {
                    *action = Some(AgentOverviewAction::RequestRemove(key.to_string()));
                }
            }
        },
    );
}

fn recent_rows<'a>(
    nodes: &'a BTreeMap<String, NodeSummary>,
    filter: &str,
) -> Vec<(String, &'a NodeSummary)> {
    let mut rows = nodes
        .values()
        .filter(|row| row_matches_filter(row, filter))
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

fn row_matches_filter(row: &NodeSummary, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }

    let needle = filter.to_ascii_lowercase();
    [
        row.node.as_str(),
        row.endpoint.as_str(),
        row.source.as_str(),
        row.service.as_str(),
        row.last_metric.as_str(),
        row.last_value.as_str(),
    ]
    .into_iter()
    .any(|value| value.to_ascii_lowercase().contains(&needle))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> NodeSummary {
        NodeSummary {
            node: "edge-alpha".to_string(),
            endpoint: "127.0.0.1:9000".to_string(),
            source: "macos".to_string(),
            service: "skid-monitor-agent".to_string(),
            metric_points: 3,
            spans: 0,
            log_records: 0,
            last_metric: "system.cpu.usage".to_string(),
            last_value: "32%".to_string(),
            last_seen: Instant::now(),
        }
    }

    #[test]
    fn agent_filter_matches_core_identity_fields() {
        let node = node();

        assert!(row_matches_filter(&node, "alpha"));
        assert!(row_matches_filter(&node, "9000"));
        assert!(row_matches_filter(&node, "MACOS"));
        assert!(row_matches_filter(&node, "cpu"));
        assert!(!row_matches_filter(&node, "postgres"));
    }
}
