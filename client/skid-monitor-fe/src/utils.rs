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
mod tests {
    use super::*;

    #[test]
    fn formats_large_numbers_without_scientific_notation() {
        assert_eq!(format_f64(17_470_000.0), "17470000");
    }

    #[test]
    fn formats_byte_metrics_as_human_readable_units() {
        assert_eq!(
            format_metric_value(139_264.0, config::METRIC_BYTE_UNIT),
            "136 KiB"
        );
        assert_eq!(
            format_metric_value(17_470_000.0, config::METRIC_BYTE_UNIT),
            "16.66 MiB"
        );
    }
}
