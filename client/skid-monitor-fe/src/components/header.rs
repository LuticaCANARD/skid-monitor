use crate::components::primitives::{status_badge, status_badge_width};
use crate::config;
use crate::model::Status;
use crate::utils::format_duration;
use eframe::egui::{self, RichText};
use std::time::Duration;

pub(crate) fn show(ui: &mut egui::Ui, compact: bool, status: &Status, uptime: Duration) {
    if compact {
        show_compact(ui, status, uptime);
    } else {
        show_wide(ui, status, uptime);
    }
}

fn show_wide(ui: &mut egui::Ui, status: &Status, uptime: Duration) {
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
    let status_rect =
        egui::Rect::from_center_size(rect.center(), egui::vec2(status_width, header_height));
    let title_rect = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(status_rect.left() - gap, rect.bottom()),
    );
    let uptime_rect = egui::Rect::from_min_max(
        egui::pos2(status_rect.right() + gap, rect.top()),
        rect.right_bottom(),
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
    status_badge(&mut status_ui, status);

    paint_uptime_label(ui, uptime_rect.right_center(), uptime);
}

fn show_compact(ui: &mut egui::Ui, status: &Status, uptime: Duration) {
    ui.horizontal_wrapped(|ui| {
        title(ui);
        status_badge(ui, status);
        uptime_label(ui, uptime);
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

fn uptime_label(ui: &mut egui::Ui, uptime: Duration) {
    ui.label(
        RichText::new(format!("uptime {}", format_duration(uptime)))
            .monospace()
            .color(config::MUTED_TEXT_COLOR),
    );
}

fn paint_uptime_label(ui: &egui::Ui, pos: egui::Pos2, uptime: Duration) {
    ui.painter().text(
        pos,
        egui::Align2::RIGHT_CENTER,
        format!("uptime {}", format_duration(uptime)),
        egui::TextStyle::Monospace.resolve(ui.style()),
        config::MUTED_TEXT_COLOR,
    );
}
