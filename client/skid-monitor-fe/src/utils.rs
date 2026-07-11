use crate::config;
use std::collections::VecDeque;
use web_time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
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

#[cfg(all(
    not(target_arch = "wasm32"),
    any(not(target_os = "linux"), feature = "high-spec")
))]
pub(crate) fn stabilize_linux_graphics_env() {}

#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
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
    let decimals = if value == 0.0 || value.fract() == 0.0 || abs >= 100.0 {
        0
    } else if abs >= 1.0 {
        2
    } else if abs >= 0.01 {
        4
    } else {
        6
    };

    trim_fraction(format!("{value:.decimals$}"))
}

pub(crate) fn format_metric_value(value: f64, unit: &str) -> String {
    if unit == config::METRIC_BYTE_UNIT {
        return format_bytes(value);
    }

    let formatted = format_f64(value);
    if unit.is_empty() {
        formatted
    } else {
        format!("{formatted} {unit}")
    }
}

fn format_bytes(value: f64) -> String {
    let mut scaled = value.abs();
    let mut unit_index = 0;
    while scaled >= config::BYTE_UNIT_BASE && unit_index + 1 < config::BYTE_DISPLAY_UNITS.len() {
        scaled /= config::BYTE_UNIT_BASE;
        unit_index += 1;
    }

    if value.is_sign_negative() {
        scaled = -scaled;
    }

    format!(
        "{} {}",
        format_f64(scaled),
        config::BYTE_DISPLAY_UNITS[unit_index]
    )
}

fn trim_fraction(mut value: String) -> String {
    if value.contains('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
    }

    if value == "-0" {
        "0".to_string()
    } else {
        value
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

pub(crate) fn format_event_time(time: SystemTime) -> String {
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return "--:--:--".to_string();
    };

    #[cfg(unix)]
    {
        if let Some(time) = local_clock_time(duration) {
            return time;
        }
    }

    utc_clock_time(duration)
}

#[cfg(unix)]
fn local_clock_time(duration: Duration) -> Option<String> {
    let seconds = libc::time_t::try_from(duration.as_secs()).ok()?;
    let mut local_time = std::mem::MaybeUninit::<libc::tm>::uninit();
    // localtime_r writes a libc tm into our stack slot for the provided epoch seconds.
    let result = unsafe { libc::localtime_r(&seconds, local_time.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }

    let local_time = unsafe { local_time.assume_init() };
    Some(format!(
        "{:02}:{:02}:{:02}",
        local_time.tm_hour, local_time.tm_min, local_time.tm_sec
    ))
}

fn utc_clock_time(duration: Duration) -> String {
    let seconds = duration.as_secs() % (24 * 60 * 60);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;

    format!("{hours:02}:{minutes:02}:{seconds:02}")
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

#[cfg(test)]
mod tests;
