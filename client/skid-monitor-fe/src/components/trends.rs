use crate::components::layout::{panel_body_height, panel_frame, sparkline_height};
use crate::components::primitives::draw_sparkline;
use crate::config;
use crate::model::MetricSample;
use crate::utils::{format_f64, shorten};
use eframe::egui::{self, RichText};
use std::collections::{BTreeMap, VecDeque};

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    max_height: f32,
    metrics: &[&MetricSample],
    metric_history: &BTreeMap<String, VecDeque<f64>>,
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        ui.heading("Trends");
        ui.separator();

        let trend_keys = visible_trend_keys(
            metrics,
            metric_history,
            if compact {
                config::TRENDS_COMPACT_VISIBLE_COUNT
            } else {
                config::TRENDS_WIDE_VISIBLE_COUNT
            },
        );
        if trend_keys.is_empty() {
            ui.label(
                RichText::new("waiting for numeric metrics").color(config::PLACEHOLDER_TEXT_COLOR),
            );
            return;
        }

        let graph_height = sparkline_height(ui.available_width());

        egui::ScrollArea::vertical()
            .id_salt("trends-scroll")
            .auto_shrink([false, true])
            .max_height(panel_body_height(inner_size.y))
            .show(ui, |ui| {
                for key in trend_keys {
                    if let Some(values) = metric_history.get(&key) {
                        let latest = values.back().copied().unwrap_or_default();
                        ui.horizontal(|ui| {
                            let name_chars = if compact {
                                config::TRENDS_COMPACT_NAME_CHARS
                            } else {
                                config::TRENDS_WIDE_NAME_CHARS
                            };
                            ui.label(RichText::new(shorten(&key, name_chars)).monospace());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_f64(latest)).monospace().strong(),
                                    );
                                },
                            );
                        });
                        draw_sparkline(ui, values, graph_height);
                        ui.add_space(config::TRENDS_ITEM_GAP);
                    }
                }
            });
    });
}

fn visible_trend_keys(
    metrics: &[&MetricSample],
    metric_history: &BTreeMap<String, VecDeque<f64>>,
    max: usize,
) -> Vec<String> {
    let mut keys = Vec::new();
    for sample in metrics.iter().rev().copied() {
        if sample.numeric.is_none() {
            continue;
        }
        if metric_history
            .get(&sample.trend_key)
            .is_none_or(|values| values.len() < config::MIN_TREND_HISTORY_POINTS)
        {
            continue;
        }
        if !keys.iter().any(|key| key == &sample.trend_key) {
            keys.push(sample.trend_key.clone());
        }
        if keys.len() >= max {
            break;
        }
    }
    keys
}
