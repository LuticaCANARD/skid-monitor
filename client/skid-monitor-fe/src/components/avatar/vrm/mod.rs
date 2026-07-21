mod animation;
mod loader;
mod renderer;
mod runtime;

use eframe::egui;
use std::sync::Arc;

pub(super) use loader::CpuVrmScene;

pub(super) fn decode(
    path: &str,
    animation_paths: &[String],
    scene_id: u64,
) -> Result<CpuVrmScene, String> {
    loader::decode(path, animation_paths, scene_id)
}

pub(super) fn install(cc: &eframe::CreationContext<'_>) -> bool {
    renderer::install(cc)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    scene: Arc<CpuVrmScene>,
    time: f32,
    expression: &str,
    crossfade_seconds: f32,
    look_yaw_degrees: f32,
    look_pitch_degrees: f32,
    spring_bone_enabled: bool,
    look_at_enabled: bool,
) {
    renderer::paint(
        painter,
        rect,
        scene,
        time,
        expression,
        crossfade_seconds,
        look_yaw_degrees,
        look_pitch_degrees,
        spring_bone_enabled,
        look_at_enabled,
    );
}

pub(super) fn clear(painter: &egui::Painter, rect: egui::Rect) {
    renderer::clear(painter, rect);
}
