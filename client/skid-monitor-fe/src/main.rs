#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    skid_monitor_fe::run_native()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
