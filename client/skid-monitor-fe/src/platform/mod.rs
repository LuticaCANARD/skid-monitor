mod ingress;

#[cfg(target_arch = "wasm32")]
pub(crate) use ingress::BrowserStorageScope;
pub(crate) use ingress::{Ingress, IngressControl, IngressMessage};

pub(crate) struct IngressUiLabels {
    pub(crate) title: &'static str,
    pub(crate) hint: &'static str,
    pub(crate) add: &'static str,
    pub(crate) remove: &'static str,
    pub(crate) empty: &'static str,
    pub(crate) add_requested: &'static str,
    pub(crate) remove_requested: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const INGRESS_UI: IngressUiLabels = IngressUiLabels {
    title: "Ingress listeners",
    hint: "127.0.0.1:9000",
    add: "Bind",
    remove: "Unbind",
    empty: "no active ingress listeners",
    add_requested: "listener bind requested",
    remove_requested: "listener removal requested",
};

#[cfg(target_arch = "wasm32")]
pub(crate) const INGRESS_UI: IngressUiLabels = IngressUiLabels {
    title: "Ingress connections",
    hint: "https://monitor.example or wss://...",
    add: "Connect",
    remove: "Disconnect",
    empty: "no active ingress connections",
    add_requested: "browser ingress connection requested",
    remove_requested: "browser ingress disconnect requested",
};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const STORAGE_TOOLTIP: &str = "SQLite state persistence is active";

#[cfg(target_arch = "wasm32")]
pub(crate) const STORAGE_TOOLTIP: &str = "Browser localStorage persistence is active";

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_native() -> eframe::Result {
    use crate::config::{APP_ID, WINDOW_INITIAL_SIZE, WINDOW_MIN_SIZE, WINDOW_TITLE};
    use eframe::egui;

    crate::utils::stabilize_linux_graphics_env();

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
        Box::new(|cc| Ok(Box::new(crate::ControlRoomApp::new(cc)))),
    )
}

#[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
fn selected_renderer() -> eframe::Renderer {
    eframe::Renderer::Wgpu
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "high-spec"),
    feature = "low-spec"
))]
fn selected_renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "high-spec"),
    not(feature = "low-spec")
))]
compile_error!("Enable either the `low-spec` or `high-spec` feature for native skid-monitor-fe.");
