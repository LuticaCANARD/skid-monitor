use crate::components::layout::{panel_body_height, panel_frame};
use crate::config;
use eframe::egui::{self, RichText};
use std::collections::BTreeMap;

pub(crate) fn show(
    ui: &mut egui::Ui,
    source_counts: &BTreeMap<String, usize>,
    panel_width: f32,
    max_height: f32,
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        ui.set_min_width(config::SOURCES_MIN_WIDTH.min(inner_size.x));
        ui.heading("Sources");
        ui.separator();
        if source_counts.is_empty() {
            ui.label(RichText::new("waiting for metrics").color(config::PLACEHOLDER_TEXT_COLOR));
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("sources-scroll")
            .auto_shrink([false, true])
            .max_height(panel_body_height(inner_size.y))
            .show(ui, |ui| {
                for (source, count) in source_counts {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(source).monospace());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(count.to_string()).strong());
                        });
                    });
                }
            });
    });
}
