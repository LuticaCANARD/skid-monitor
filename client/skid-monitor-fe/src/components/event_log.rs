use crate::components::layout::{panel_body_height, panel_frame};
use crate::components::primitives::kind_color;
use crate::config;
use crate::model::EventRow;
use crate::utils::shorten;
use eframe::egui::{self, RichText};

pub(crate) fn show(ui: &mut egui::Ui, panel_width: f32, max_height: f32, events: &[&EventRow]) {
    panel_frame(ui, panel_width, max_height, |ui, inner_size| {
        ui.heading("Event Log");
        ui.separator();
        if events.is_empty() {
            ui.label(RichText::new("no events yet").color(config::PLACEHOLDER_TEXT_COLOR));
            return;
        }

        let spacing = ui.spacing().item_spacing.x;
        let message_width = (inner_size.x
            - config::EVENT_LOG_TIME_WIDTH
            - config::EVENT_LOG_KIND_WIDTH
            - spacing * 2.0)
            .max(120.0);

        egui::ScrollArea::vertical()
            .id_salt("event-log-scroll")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(panel_body_height(inner_size.y))
            .show(ui, |ui| {
                for event in events.iter().copied() {
                    ui.horizontal(|ui| {
                        left_aligned_cell(
                            ui,
                            [config::EVENT_LOG_TIME_WIDTH, config::EVENT_LOG_ROW_HEIGHT],
                            egui::Label::new(
                                RichText::new(&event.time)
                                    .monospace()
                                    .color(config::PLACEHOLDER_TEXT_COLOR),
                            )
                            .halign(egui::Align::Min),
                        );
                        left_aligned_cell(
                            ui,
                            [config::EVENT_LOG_KIND_WIDTH, config::EVENT_LOG_ROW_HEIGHT],
                            egui::Label::new(
                                RichText::new(&event.kind)
                                    .monospace()
                                    .color(kind_color(&event.kind)),
                            )
                            .halign(egui::Align::Min),
                        );
                        let message = shorten(&event.message, event_message_chars(message_width));
                        let response = left_aligned_cell(
                            ui,
                            [message_width, config::EVENT_LOG_ROW_HEIGHT],
                            egui::Label::new(message.clone())
                                .truncate()
                                .halign(egui::Align::Min),
                        );
                        if message != event.message {
                            response.on_hover_text(&event.message);
                        }
                    });
                }
            });
    });
}

fn left_aligned_cell(
    ui: &mut egui::Ui,
    size: impl Into<egui::Vec2>,
    label: egui::Label,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        size.into(),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| ui.add(label),
    )
    .inner
}

fn event_message_chars(width: f32) -> usize {
    (width / 7.5).floor().clamp(18.0, 180.0) as usize
}
