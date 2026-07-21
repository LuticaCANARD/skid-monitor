use crate::components::primitives::{
    alert_badge, alert_badge_width, status_badge, status_badge_width, summary_chip,
};
use crate::config;
use crate::model::{AlertSummary, OperationalSummary, Status};
use crate::platform::STORAGE_TOOLTIP;
use eframe::egui::{self, Color32, RichText};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HeaderAction {
    #[default]
    None,
    OpenCharacter,
    OpenSettings,
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    status: &Status,
    alert_summary: AlertSummary,
    operational_summary: OperationalSummary,
) -> HeaderAction {
    if compact {
        show_compact(ui, status, alert_summary, operational_summary)
    } else {
        show_wide(ui, status, alert_summary, operational_summary)
    }
}

fn show_wide(
    ui: &mut egui::Ui,
    status: &Status,
    alert_summary: AlertSummary,
    operational_summary: OperationalSummary,
) -> HeaderAction {
    let mut action = HeaderAction::None;
    let available_width = ui.available_width().max(1.0);
    let header_height = (config::APP_TITLE_SIZE
        + config::APP_SUBTITLE_SIZE
        + config::HEADER_SUMMARY_CHIP_HEIGHT
        + ui.spacing().item_spacing.y
        + config::HEADER_SUMMARY_GAP)
        .max(ui.spacing().interact_size.y);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, header_height),
        egui::Sense::hover(),
    );
    let gap = ui.spacing().item_spacing.x;
    let status_width = status_badge_width(ui, status);
    let alert_width = alert_badge_width(ui, alert_summary);
    let badge_width = status_width
        + gap
        + alert_width
        + gap
        + config::CHARACTER_BUTTON_WIDTH
        + gap
        + config::SETTINGS_BUTTON_WIDTH;
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
    title(&mut title_ui, operational_summary);

    let mut status_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("header-status")
            .max_rect(status_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    status_ui.spacing_mut().item_spacing.x = gap;
    status_badge(&mut status_ui, status);
    alert_badge(&mut status_ui, alert_summary);
    if status_ui
        .add_sized(
            [
                config::CHARACTER_BUTTON_WIDTH,
                status_ui.spacing().interact_size.y,
            ],
            egui::Button::new("Character"),
        )
        .clicked()
    {
        action = HeaderAction::OpenCharacter;
    }
    if status_ui
        .add_sized(
            [
                config::SETTINGS_BUTTON_WIDTH,
                status_ui.spacing().interact_size.y,
            ],
            egui::Button::new("Settings"),
        )
        .clicked()
    {
        action = HeaderAction::OpenSettings;
    }

    action
}

fn show_compact(
    ui: &mut egui::Ui,
    status: &Status,
    alert_summary: AlertSummary,
    operational_summary: OperationalSummary,
) -> HeaderAction {
    let mut action = HeaderAction::None;
    title(ui, operational_summary);
    ui.add_space(config::HEADER_SUMMARY_GAP);
    ui.horizontal_wrapped(|ui| {
        status_badge(ui, status);
        alert_badge(ui, alert_summary);
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Character").clicked() {
            action = HeaderAction::OpenCharacter;
        }
        if ui.button("Settings").clicked() {
            action = HeaderAction::OpenSettings;
        }
    });
    action
}

fn title(ui: &mut egui::Ui, summary: OperationalSummary) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(config::APP_TITLE)
                .size(config::APP_TITLE_SIZE)
                .strong()
                .color(ui.visuals().strong_text_color()),
        );
        ui.label(
            RichText::new(config::APP_SUBTITLE)
                .size(config::APP_SUBTITLE_SIZE)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(config::HEADER_SUMMARY_GAP);
        summary_strip(ui, summary);
    });
}

fn summary_strip(ui: &mut egui::Ui, summary: OperationalSummary) {
    ui.horizontal_wrapped(|ui| {
        summary_chip(
            ui,
            format!("agents {}", summary.agents),
            ui.visuals().weak_text_color(),
            Some(format!("{} registered observation agents", summary.agents)),
        );
        summary_chip(
            ui,
            format!("listeners {}", summary.listeners),
            config::EVENT_METRICS_COLOR,
            Some(format!(
                "{} active client ingress listeners",
                summary.listeners
            )),
        );
        summary_chip(
            ui,
            format!("online {}", summary.online),
            config::STATUS_LISTENING_COLOR,
            Some(format!("{} agents have reported signals", summary.online)),
        );
        if summary.pending > 0 {
            summary_chip(
                ui,
                format!("pending {}", summary.pending),
                config::MUTED_TEXT_COLOR,
                Some(format!(
                    "{} registered agents have not sent signals",
                    summary.pending
                )),
            );
        }
        if summary.warning > 0 {
            summary_chip(
                ui,
                format!("warning {}", summary.warning),
                config::ALERT_WARNING_COLOR,
                Some(format!("{} agents have warning alerts", summary.warning)),
            );
        }
        if summary.critical > 0 {
            summary_chip(
                ui,
                format!("critical {}", summary.critical),
                config::ALERT_CRITICAL_COLOR,
                Some(format!("{} agents have critical alerts", summary.critical)),
            );
        }
        let (storage_label, storage_color, tooltip) = if summary.storage_enabled {
            (
                "state saved",
                config::STATUS_LISTENING_COLOR,
                STORAGE_TOOLTIP,
            )
        } else {
            (
                "volatile state",
                Color32::from_rgb(210, 168, 74),
                "State persistence is disabled for this session",
            )
        };
        summary_chip(ui, storage_label, storage_color, Some(tooltip.to_string()));
    });
}
