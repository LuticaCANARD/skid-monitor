use crate::components::primitives::{
    alert_badge, alert_badge_width, status_badge, status_badge_width,
};
use crate::config;
use crate::model::{AlertSummary, Status};
use eframe::egui::{self, RichText};

pub(crate) fn show(ui: &mut egui::Ui, compact: bool, status: &Status, alert_summary: AlertSummary) {
    if compact {
        show_compact(ui, status, alert_summary);
    } else {
        show_wide(ui, status, alert_summary);
    }
}

fn show_wide(ui: &mut egui::Ui, status: &Status, alert_summary: AlertSummary) {
    let available_width = ui.available_width().max(1.0);
    let header_height =
        (config::APP_TITLE_SIZE + config::APP_SUBTITLE_SIZE + ui.spacing().item_spacing.y)
            .max(ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, header_height),
        egui::Sense::hover(),
    );
    let gap = ui.spacing().item_spacing.x;
    let status_width = status_badge_width(ui, status);
    let alert_width = alert_badge_width(ui, alert_summary);
    let badge_width = status_width + gap + alert_width;
    let status_rect = egui::Rect::from_min_size(
        egui::pos2((rect.right() - badge_width).max(rect.left()), rect.top()),
        egui::vec2(badge_width, header_height),
    );
    let title_rect = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(status_rect.left() - gap, rect.bottom()),
    );

    let mut title_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("header-title")
            .max_rect(title_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    title(&mut title_ui);

    let mut status_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("header-status")
            .max_rect(status_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    status_ui.spacing_mut().item_spacing.x = gap;
    status_badge(&mut status_ui, status);
    alert_badge(&mut status_ui, alert_summary);
}

fn show_compact(ui: &mut egui::Ui, status: &Status, alert_summary: AlertSummary) {
    ui.horizontal_wrapped(|ui| {
        title(ui);
        status_badge(ui, status);
        alert_badge(ui, alert_summary);
    });
}

fn title(ui: &mut egui::Ui) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(config::APP_TITLE)
                .size(config::APP_TITLE_SIZE)
                .strong()
                .color(config::TITLE_COLOR),
        );
        ui.label(
            RichText::new(config::APP_SUBTITLE)
                .size(config::APP_SUBTITLE_SIZE)
                .color(config::MUTED_TEXT_COLOR),
        );
    });
}
