mod app;
mod components;
mod config;
mod model;
mod signal;
mod utils;

use app::ControlRoomApp;
use config::{APP_ID, WINDOW_INITIAL_SIZE, WINDOW_MIN_SIZE, WINDOW_TITLE};
use eframe::egui;

fn main() -> eframe::Result {
    utils::stabilize_linux_graphics_env();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(WINDOW_INITIAL_SIZE)
            .with_min_inner_size(WINDOW_MIN_SIZE),
        renderer: eframe::Renderer::Glow,
        run_and_return: false,
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        native_options,
        Box::new(|cc| Ok(Box::new(ControlRoomApp::new(cc)))),
    )
}
