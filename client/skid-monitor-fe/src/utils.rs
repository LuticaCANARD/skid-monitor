use crate::config;
use std::collections::VecDeque;
use std::time::Duration;

#[cfg(target_os = "linux")]
pub(crate) fn stabilize_linux_graphics_env() {
    // Set before eframe/glutin starts any graphics threads.
    unsafe {
        if !matches!(
            std::env::var(config::USE_GPU_ENV).as_deref(),
            Ok(config::ENABLED_ENV_VALUE)
        ) {
            std::env::set_var(config::LIBGL_ALWAYS_SOFTWARE_ENV, config::SOFTWARE_GL_VALUE);
            std::env::set_var(
                config::MESA_LOADER_DRIVER_OVERRIDE_ENV,
                config::SOFTWARE_DRIVER_VALUE,
            );
            if matches!(
                std::env::var(config::GALLIUM_DRIVER_ENV).as_deref(),
                Ok(config::ZINK_DRIVER_VALUE)
            ) {
                std::env::set_var(config::GALLIUM_DRIVER_ENV, config::SOFTWARE_DRIVER_VALUE);
            }
        }

        if prefers_x11_backend() {
            std::env::remove_var(config::WAYLAND_DISPLAY_ENV);
            std::env::remove_var(config::WAYLAND_SOCKET_ENV);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn stabilize_linux_graphics_env() {}

#[cfg(target_os = "linux")]
fn prefers_x11_backend() -> bool {
    let use_wayland = matches!(
        std::env::var(config::USE_WAYLAND_ENV).as_deref(),
        Ok(config::ENABLED_ENV_VALUE)
    );
    let has_x11 = std::env::var(config::DISPLAY_ENV)
        .map(|display| !display.is_empty())
        .unwrap_or(false);

    has_x11 && !use_wayland
}

pub(crate) fn push_capped<T>(items: &mut VecDeque<T>, item: T, max: usize) {
    if items.len() >= max {
        items.pop_front();
    }
    items.push_back(item);
}

pub(crate) fn format_f64(value: f64) -> String {
    let abs = value.abs();
    if value.fract() == 0.0 && abs < 1_000_000.0 {
        format!("{value:.0}")
    } else if !(0.01..1_000_000.0).contains(&abs) && value != 0.0 {
        format!("{value:.3e}")
    } else {
        format!("{value:.2}")
    }
}

pub(crate) fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

pub(crate) fn shorten(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}
