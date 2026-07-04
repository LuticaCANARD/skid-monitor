use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::kind_color;
use crate::config;
use crate::model::EventRow;
use crate::utils::format_duration;
use eframe::egui::{self, RichText};
use std::collections::VecDeque;

pub(crate) fn show(
    ui: &mut egui::Ui,
    panel_width: f32,
    max_height: f32,
    events: &VecDeque<EventRow>,
) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        ui.heading("Event Log");
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("event-log-scroll")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(panel_body_height(inner_size.y))
            .show(ui, |ui| {
                for event in events {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format_duration(event.at.elapsed()))
                                .monospace()
                                .color(config::PLACEHOLDER_TEXT_COLOR),
                        );
                        ui.label(
                            RichText::new(&event.kind)
                                .monospace()
                                .color(kind_color(&event.kind)),
                        );
                        ui.label(&event.message);
                    });
                }
            });
    });
}
