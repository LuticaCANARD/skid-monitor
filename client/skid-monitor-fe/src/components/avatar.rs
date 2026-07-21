mod model;
mod state;
#[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
mod vrm;

use crate::alert::AlertStore;
use crate::components::layout::{panel_body_height, panel_frame};
use crate::model::{AvatarMotion, AvatarReactionProfile, NodeSummary};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Stroke};
pub(crate) use model::AvatarModelCache;
use state::AvatarAlertState;
use std::time::Duration;

#[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
pub(crate) fn install_vrm_renderer(cc: &eframe::CreationContext<'_>) -> bool {
    vrm::install(cc)
}

const MOTION_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const PULSE_SCALE: f32 = 0.04;
const BOUNCE_DISTANCE: f32 = 12.0;
const SHAKE_DISTANCE: f32 = 6.0;

pub(crate) struct AvatarPresenterInput {
    state: AvatarAlertState,
    model_name: String,
    message: String,
    motion: AvatarMotion,
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    expression: String,
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    animation_crossfade_seconds: f32,
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    spring_bone_enabled: bool,
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    look_at_enabled: bool,
    active_alert_count: usize,
}

impl AvatarPresenterInput {
    pub(crate) fn for_node(
        nodes: &[&NodeSummary],
        alerts: &AlertStore,
        profile: &AvatarReactionProfile,
    ) -> Self {
        let node = nodes.first();
        let severity =
            node.and_then(|node| alerts.highest_for_presenter(&node.endpoint, &node.node));
        let state = AvatarAlertState::from_severity(severity);
        let action = profile.action_for(severity);
        let active_alert_count = node
            .map(|node| alerts.active_count_for_presenter(&node.endpoint, &node.node))
            .unwrap_or(0);

        Self {
            state,
            model_name: profile.model_name.clone(),
            message: action.message.clone(),
            motion: action.motion,
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            expression: action.expression.clone(),
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            animation_crossfade_seconds: profile.animation_crossfade_seconds,
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            spring_bone_enabled: profile.spring_bone_enabled,
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            look_at_enabled: profile.look_at_enabled,
            active_alert_count,
        }
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    panel_width: f32,
    panel_height: f32,
    input: AvatarPresenterInput,
    model: &AvatarModelCache,
) {
    panel_frame(ui, panel_width, panel_height, |ui, inner_size| {
        ui.horizontal(|ui| {
            ui.heading("Character");
            ui.label(RichText::new(input.state.label()).color(input.state.accent()));
            ui.label(
                RichText::new(input.motion.label())
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            if model.loading() {
                ui.spinner();
                ui.label(
                    RichText::new("loading model")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            } else if let Some(error) = model.error() {
                ui.label(
                    RichText::new("built-in fallback")
                        .small()
                        .color(input.state.accent()),
                )
                .on_hover_text(error);
            } else if let Some(label) = model.loaded_label() {
                ui.label(
                    RichText::new(label)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if let Some(animation) = model.animation_label() {
                    ui.label(
                        RichText::new(format!("+ {animation}"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
            }
            ui.add(egui::Label::new(&input.model_name).truncate())
                .on_hover_text(&input.model_name);
        });
        ui.separator();

        let viewport_height = panel_body_height(inner_size.y);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(inner_size.x, viewport_height),
            egui::Sense::hover(),
        );
        let time = ui.input(|input| input.time);
        if input.motion != AvatarMotion::Still {
            ui.ctx().request_repaint_after(MOTION_FRAME_INTERVAL);
        }
        let painter = ui.painter().with_clip_rect(rect);
        let character_rect = motion_rect(rect, input.motion, time);
        #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
        if let Some(scene) = model.vrm_scene() {
            if scene.needs_continuous_update() {
                ui.ctx().request_repaint_after(MOTION_FRAME_INTERVAL);
            }
            let (look_yaw_degrees, look_pitch_degrees) = ui
                .ctx()
                .pointer_hover_pos()
                .filter(|pointer| rect.contains(*pointer))
                .map_or((0.0, 0.0), |pointer| {
                    let horizontal =
                        ((pointer.x - rect.center().x) / (rect.width() * 0.5)).clamp(-1.0, 1.0);
                    let vertical =
                        ((pointer.y - rect.center().y) / (rect.height() * 0.5)).clamp(-1.0, 1.0);
                    (-horizontal * 45.0, vertical * 30.0)
                });
            vrm::paint(
                &painter,
                character_rect.shrink(12.0),
                scene.clone(),
                time as f32,
                &input.expression,
                input.animation_crossfade_seconds,
                look_yaw_degrees,
                look_pitch_degrees,
                input.spring_bone_enabled,
                input.look_at_enabled,
            );
        } else {
            vrm::clear(&painter, character_rect);
            if let Some(image) = model.image() {
                paint_sprite(&painter, character_rect, image);
            } else {
                paint_placeholder(&painter, character_rect, &input);
            }
        }
        #[cfg(not(all(not(target_arch = "wasm32"), feature = "high-spec")))]
        if let Some(image) = model.image() {
            paint_sprite(&painter, character_rect, image);
        } else {
            paint_placeholder(&painter, character_rect, &input);
        }
        paint_message(&painter, rect, &input);
    });
}

fn motion_rect(rect: egui::Rect, motion: AvatarMotion, time: f64) -> egui::Rect {
    let seconds = time as f32;
    match motion {
        AvatarMotion::Still => rect,
        AvatarMotion::Pulse => {
            let scale = 1.0 + PULSE_SCALE * (seconds * std::f32::consts::TAU).sin();
            egui::Rect::from_center_size(rect.center(), rect.size() * scale)
        }
        AvatarMotion::Bounce => {
            let offset = -BOUNCE_DISTANCE * (seconds * std::f32::consts::TAU * 1.5).sin().abs();
            rect.translate(egui::vec2(0.0, offset))
        }
        AvatarMotion::Shake => {
            let offset = SHAKE_DISTANCE * (seconds * std::f32::consts::TAU * 8.0).sin();
            rect.translate(egui::vec2(offset, 0.0))
        }
    }
}

fn paint_sprite(painter: &egui::Painter, bounds: egui::Rect, image: &model::AvatarModelImage) {
    let rect = contain_rect(bounds.shrink(12.0), image.size());
    painter.image(
        image.texture_id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn contain_rect(bounds: egui::Rect, source_size: egui::Vec2) -> egui::Rect {
    if bounds.width() <= 0.0
        || bounds.height() <= 0.0
        || source_size.x <= 0.0
        || source_size.y <= 0.0
    {
        return egui::Rect::from_center_size(bounds.center(), egui::Vec2::ZERO);
    }

    let scale = (bounds.width() / source_size.x).min(bounds.height() / source_size.y);
    egui::Rect::from_center_size(bounds.center(), source_size * scale)
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
}

fn paint_message(painter: &egui::Painter, rect: egui::Rect, input: &AvatarPresenterInput) {
    let accent = input.state.accent();
    let bubble_width = (rect.width() - 28.0).clamp(1.0, 310.0);
    let message_galley = painter.layout(
        input.message.clone(),
        FontId::proportional(14.0),
        Color32::WHITE,
        (bubble_width - 24.0).max(1.0),
    );
    let available_height = (rect.height() - 28.0).max(1.0);
    let bubble_height = (message_galley.size().y + 36.0)
        .clamp(1.0, 112.0)
        .min(available_height);
    let bubble = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 14.0, rect.top() + 14.0),
        egui::vec2(bubble_width, bubble_height),
    );
    painter.rect(
        bubble,
        8.0,
        Color32::from_black_alpha(180),
        Stroke::new(1.0, accent),
        egui::StrokeKind::Inside,
    );
    let message_origin = bubble.left_top() + egui::vec2(12.0, 10.0);
    let message_clip = egui::Rect::from_min_max(
        message_origin,
        egui::pos2(
            (bubble.right() - 12.0).max(message_origin.x),
            (bubble.bottom() - 24.0).max(message_origin.y),
        ),
    );
    painter
        .with_clip_rect(message_clip)
        .galley(message_origin, message_galley, Color32::WHITE);
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
        let profile = AvatarReactionProfile::default();
        let input = AvatarPresenterInput::for_node(&[], &AlertStore::default(), &profile);

        assert_eq!(input.state, AvatarAlertState::Idle);
        assert_eq!(input.model_name, "Skid");
        assert_eq!(input.message, "All systems look calm.");
        assert_eq!(input.motion, AvatarMotion::Still);
        assert_eq!(input.active_alert_count, 0);
    }

    #[test]
    fn presenter_uses_the_configured_idle_reaction() {
        let mut profile = AvatarReactionProfile {
            model_name: "Operator Cat".to_string(),
            ..AvatarReactionProfile::default()
        };
        profile.idle.message = "Watching the racks.".to_string();
        profile.idle.motion = AvatarMotion::Bounce;

        let input = AvatarPresenterInput::for_node(&[], &AlertStore::default(), &profile);

        assert_eq!(input.model_name, "Operator Cat");
        assert_eq!(input.message, "Watching the racks.");
        assert_eq!(input.motion, AvatarMotion::Bounce);
    }

    #[test]
    fn receiver_failure_uses_the_configured_critical_reaction() {
        let mut alerts = AlertStore::default();
        alerts.observe_receiver_error("127.0.0.1:9000", "listener failed");
        let node = NodeSummary {
            node: "agent-a".to_string(),
            endpoint: "127.0.0.1:9000".to_string(),
            source: "agent".to_string(),
            service: "skid-monitor-agent".to_string(),
            metric_points: 0,
            spans: 0,
            log_records: 0,
            last_metric: String::new(),
            last_value: String::new(),
            last_seen: web_time::Instant::now(),
        };
        let mut profile = AvatarReactionProfile::default();
        profile.critical.message = "The receiver is down.".to_string();
        profile.critical.motion = AvatarMotion::Bounce;

        let input = AvatarPresenterInput::for_node(&[&node], &alerts, &profile);

        assert_eq!(input.state, AvatarAlertState::Urgent);
        assert_eq!(input.message, "The receiver is down.");
        assert_eq!(input.motion, AvatarMotion::Bounce);
        assert_eq!(input.active_alert_count, 1);
    }

    #[test]
    fn still_motion_does_not_change_the_character_rect() {
        let rect = test_rect();

        assert_eq!(motion_rect(rect, AvatarMotion::Still, 123.0), rect);
    }

    #[test]
    fn pulse_motion_scales_around_the_center() {
        let rect = test_rect();
        let transformed = motion_rect(rect, AvatarMotion::Pulse, 0.25);

        assert_eq!(transformed.center(), rect.center());
        assert!(transformed.width() > rect.width());
        assert!(transformed.height() > rect.height());
    }

    #[test]
    fn bounce_motion_moves_up_without_resizing() {
        let rect = test_rect();
        let transformed = motion_rect(rect, AvatarMotion::Bounce, 1.0 / 6.0);

        assert_eq!(transformed.size(), rect.size());
        assert_eq!(transformed.center().x, rect.center().x);
        assert!(transformed.center().y < rect.center().y);
    }

    #[test]
    fn shake_motion_moves_sideways_without_resizing() {
        let rect = test_rect();
        let transformed = motion_rect(rect, AvatarMotion::Shake, 1.0 / 32.0);

        assert_eq!(transformed.size(), rect.size());
        assert!(transformed.center().x > rect.center().x);
        assert_eq!(transformed.center().y, rect.center().y);
    }

    #[test]
    fn sprite_containment_preserves_aspect_ratio() {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0));
        let rect = contain_rect(bounds, egui::vec2(50.0, 100.0));

        assert_eq!(rect.height(), 100.0);
        assert_eq!(rect.width(), 50.0);
        assert_eq!(rect.center(), bounds.center());
    }

    fn test_rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 100.0))
    }
}
