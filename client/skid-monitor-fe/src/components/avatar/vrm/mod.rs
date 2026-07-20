mod loader;
mod renderer;

use eframe::egui;
use std::sync::Arc;

pub(super) use loader::CpuVrmScene;

pub(super) fn decode(path: &str, scene_id: u64) -> Result<CpuVrmScene, String> {
    loader::decode(path, scene_id)
}

pub(super) fn install(cc: &eframe::CreationContext<'_>) -> bool {
    renderer::install(cc)
}

pub(super) fn paint(painter: &egui::Painter, rect: egui::Rect, scene: Arc<CpuVrmScene>) {
    renderer::paint(painter, rect, scene);
}

pub(super) fn clear(painter: &egui::Painter, rect: egui::Rect) {
    renderer::clear(painter, rect);
}
