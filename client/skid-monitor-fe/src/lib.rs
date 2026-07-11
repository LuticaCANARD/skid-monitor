mod alert;
mod app;
mod components;
mod config;
mod edge;
mod model;
mod pages;
mod platform;
mod signal;
mod state;
mod storage;
mod ui_settings;
mod utils;
mod view;

use app::ControlRoomApp;

#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
compile_error!("Enable the `web` feature when building skid-monitor-fe for wasm32.");

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    platform::run_native()
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// JavaScript handle for the browser-hosted control-room frontend.
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        eframe::WebLogger::init(log::LevelFilter::Info).ok();
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    pub async fn start(
        &self,
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<(), wasm_bindgen::JsValue> {
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(ControlRoomApp::new(cc)))),
            )
            .await
    }

    pub fn destroy(&self) {
        self.runner.destroy();
    }

    pub fn has_panicked(&self) -> bool {
        self.runner.has_panicked()
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}
