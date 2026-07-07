mod alert;
mod app;
mod components;
mod config;
mod edge;
mod model;
mod pages;
mod signal;
mod state;
mod storage;
mod ui_settings;
mod utils;
mod view;

use app::ControlRoomApp;
use config::{APP_ID, WINDOW_INITIAL_SIZE, WINDOW_MIN_SIZE, WINDOW_TITLE};
use eframe::egui;

#[cfg(not(any(feature = "low-spec", feature = "high-spec")))]
compile_error!("Enable either the `low-spec` or `high-spec` feature for skid-monitor-fe.");

fn main() -> eframe::Result {
    utils::stabilize_linux_graphics_env();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(WINDOW_INITIAL_SIZE)
            .with_min_inner_size(WINDOW_MIN_SIZE),
        renderer: selected_renderer(),
        run_and_return: false,
        ..Default::default()
    };

    eframe::run_native(
        APP_ID,
        native_options,
        Box::new(|cc| Ok(Box::new(ControlRoomApp::new(cc)))),
    )
}

#[cfg(feature = "high-spec")]
fn selected_renderer() -> eframe::Renderer {
    eframe::Renderer::Wgpu
}

#[cfg(all(not(feature = "high-spec"), feature = "low-spec"))]
fn selected_renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}
