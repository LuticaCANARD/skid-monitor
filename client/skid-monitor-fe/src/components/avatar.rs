use crate::alert::AlertStore;
use crate::components::layout::{panel_body_height, panel_frame};
use crate::config;
use crate::model::{AlertSeverity, NodeSummary};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AvatarAlertState {
    Idle,
    Concerned,
    Urgent,
}

impl AvatarAlertState {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Concerned => "Concerned",
            Self::Urgent => "Urgent",
        }
    }

    fn accent(self) -> Color32 {
        match self {
            Self::Idle => config::ALERT_CLEAR_COLOR,
            Self::Concerned => config::ALERT_WARNING_COLOR,
            Self::Urgent => config::ALERT_CRITICAL_COLOR,
        }
    }
}

pub(crate) struct AvatarPresenterInput {
    state: AvatarAlertState,
    message: String,
    active_alert_count: usize,
}

impl AvatarPresenterInput {
    pub(crate) fn for_node(nodes: &[&NodeSummary], alerts: &AlertStore) -> Self {
        let severity = nodes
            .first()
            .and_then(|node| alerts.highest_for_node(&node.endpoint, &node.node));
        let state = match severity {
            Some(AlertSeverity::Critical) => AvatarAlertState::Urgent,
            Some(AlertSeverity::Warning) => AvatarAlertState::Concerned,
            None => AvatarAlertState::Idle,
        };
        let active_alert_count = alerts.summary().active_count;
        let message = match state {
            AvatarAlertState::Idle => "All systems look calm.".to_string(),
            AvatarAlertState::Concerned => "A warning needs attention.".to_string(),
            AvatarAlertState::Urgent => "Critical condition detected!".to_string(),
        };

        Self {
            state,
            message,
            active_alert_count,
        }
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    panel_width: f32,
    panel_height: f32,
    input: AvatarPresenterInput,
) {
    panel_frame(ui, panel_width, panel_height, |ui, inner_size| {
        ui.horizontal(|ui| {
            ui.heading("Character");
            ui.label(RichText::new(input.state.label()).color(input.state.accent()));
        });
        ui.separator();

        let viewport_height = panel_body_height(inner_size.y);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(inner_size.x, viewport_height),
            egui::Sense::hover(),
        );
        paint_placeholder(ui.painter(), rect, &input);
    });
}

fn paint_placeholder(painter: &egui::Painter, rect: egui::Rect, input: &AvatarPresenterInput) {
    let accent = input.state.accent();
    let center = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.08);
    let scale = rect.width().min(rect.height()).clamp(120.0, 340.0) / 340.0;
    let head_radius = 46.0 * scale;
    let head_center = center - egui::vec2(0.0, 90.0 * scale);
    let body_top = head_center + egui::vec2(0.0, head_radius * 0.82);
    let body_bottom = center + egui::vec2(0.0, 105.0 * scale);
    let line = Stroke::new((10.0 * scale).max(4.0), accent);

    painter.line_segment([body_top, body_bottom], line);
    painter.line_segment(
        [
            center - egui::vec2(68.0 * scale, 5.0 * scale),
            center + egui::vec2(68.0 * scale, 5.0 * scale),
        ],
        line,
    );
    painter.line_segment(
        [body_bottom, body_bottom + egui::vec2(-50.0, 72.0) * scale],
        line,
    );
    painter.line_segment(
        [body_bottom, body_bottom + egui::vec2(50.0, 72.0) * scale],
        line,
    );
    painter.circle_filled(head_center, head_radius, Color32::from_rgb(225, 205, 188));
    painter.circle_stroke(head_center, head_radius, Stroke::new(3.0, accent));

    let eye_y = head_center.y - 8.0 * scale;
    for x in [-16.0, 16.0] {
        painter.circle_filled(
            egui::pos2(head_center.x + x * scale, eye_y),
            (4.0 * scale).max(2.0),
            Color32::from_rgb(32, 36, 44),
        );
    }
    let mouth_y = head_center.y + 17.0 * scale;
    match input.state {
        AvatarAlertState::Idle => {
            painter.line_segment(
                [
                    egui::pos2(head_center.x - 12.0 * scale, mouth_y),
                    egui::pos2(head_center.x + 12.0 * scale, mouth_y),
                ],
                Stroke::new(2.0, Color32::from_rgb(55, 45, 45)),
            );
        }
        AvatarAlertState::Concerned => {
            painter.circle_stroke(
                egui::pos2(head_center.x, mouth_y + 7.0 * scale),
                10.0 * scale,
                Stroke::new(2.0, Color32::from_rgb(55, 45, 45)),
            );
        }
        AvatarAlertState::Urgent => {
            painter.circle_filled(
                egui::pos2(head_center.x, mouth_y),
                11.0 * scale,
                Color32::from_rgb(95, 30, 35),
            );
        }
    }

    let bubble = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 14.0, rect.top() + 14.0),
        egui::vec2((rect.width() - 28.0).min(310.0), 62.0),
    );
    painter.rect(
        bubble,
        8.0,
        Color32::from_black_alpha(180),
        Stroke::new(1.0, accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        bubble.left_top() + egui::vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        &input.message,
        FontId::proportional(14.0),
        Color32::WHITE,
    );
    painter.text(
        bubble.left_bottom() + egui::vec2(12.0, -10.0),
        Align2::LEFT_BOTTOM,
        format!("{} active alerts", input.active_alert_count),
        FontId::monospace(11.0),
        accent,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_node_alert_maps_to_idle() {
        let input = AvatarPresenterInput::for_node(&[], &AlertStore::default());

        assert_eq!(input.state, AvatarAlertState::Idle);
        assert_eq!(input.active_alert_count, 0);
    }
}
