use super::*;

#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
#[test]
fn linux_render_mode_prefers_gpu_unless_software_is_explicit() {
    assert_eq!(linux_render_mode(None, None), LinuxRenderMode::Auto);
    assert_eq!(linux_render_mode(Some("auto"), None), LinuxRenderMode::Auto);
    assert_eq!(
        linux_render_mode(Some("software"), None),
        LinuxRenderMode::Software
    );
    assert_eq!(
        linux_render_mode(None, Some("0")),
        LinuxRenderMode::Software
    );
}

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

#[test]
fn formats_event_time_as_fixed_clock_time() {
    let time = format_event_time(UNIX_EPOCH + Duration::from_secs(3_723));

    assert_eq!(time.len(), 8);
    assert_eq!(time.chars().filter(|ch| *ch == ':').count(), 2);
}
