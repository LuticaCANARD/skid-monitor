use eframe::egui::{Color32, Vec2};

pub(crate) const APP_ID: &str = "skid-monitor-control-room";
pub(crate) const WINDOW_TITLE: &str = "Skid Monitor Control Room";
pub(crate) const APP_TITLE: &str = "Skid Monitor";
pub(crate) const APP_SUBTITLE: &str = "control room";
pub(crate) const WINDOW_INITIAL_SIZE: [f32; 2] = [1180.0, 760.0];
pub(crate) const WINDOW_MIN_SIZE: [f32; 2] = [280.0, 640.0];
pub(crate) const REPAINT_INTERVAL_MS: u64 = 250;

#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const USE_GPU_ENV: &str = "SKID_MONITOR_FE_USE_GPU";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const USE_WAYLAND_ENV: &str = "SKID_MONITOR_FE_USE_WAYLAND";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const ENABLED_ENV_VALUE: &str = "1";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const LIBGL_ALWAYS_SOFTWARE_ENV: &str = "LIBGL_ALWAYS_SOFTWARE";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const MESA_LOADER_DRIVER_OVERRIDE_ENV: &str = "MESA_LOADER_DRIVER_OVERRIDE";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const GALLIUM_DRIVER_ENV: &str = "GALLIUM_DRIVER";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const SOFTWARE_GL_VALUE: &str = "1";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const SOFTWARE_DRIVER_VALUE: &str = "llvmpipe";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const ZINK_DRIVER_VALUE: &str = "zink";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const WAYLAND_DISPLAY_ENV: &str = "WAYLAND_DISPLAY";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const WAYLAND_SOCKET_ENV: &str = "WAYLAND_SOCKET";
#[cfg(all(target_os = "linux", not(feature = "high-spec")))]
pub(crate) const DISPLAY_ENV: &str = "DISPLAY";

pub(crate) const MAX_EVENTS: usize = 256;
pub(crate) const MAX_METRICS: usize = 512;
pub(crate) const MAX_HISTORY_POINTS: usize = 180;

pub(crate) const CONTENT_MAX_WIDTH: f32 = 1320.0;
pub(crate) const CONTENT_SIDE_MARGIN_MIN: f32 = 12.0;
pub(crate) const CONTENT_SIDE_MARGIN_MAX: f32 = 72.0;
pub(crate) const CONTENT_SIDE_MARGIN_RATIO: f32 = 0.035;
pub(crate) const CONTENT_BOTTOM_MARGIN_MIN: f32 = 18.0;
pub(crate) const CONTENT_BOTTOM_MARGIN_MAX: f32 = 56.0;
pub(crate) const CONTENT_BOTTOM_MARGIN_RATIO: f32 = 0.04;
pub(crate) const CONTENT_FRAME_MARGIN: i8 = 18;

pub(crate) const GLOBAL_ITEM_SPACING: Vec2 = Vec2::new(10.0, 8.0);
pub(crate) const GLOBAL_BUTTON_PADDING: Vec2 = Vec2::new(10.0, 6.0);
pub(crate) const SECTION_GAP: f32 = 12.0;
pub(crate) const HEADER_COUNTER_GAP: f32 = 16.0;
pub(crate) const PANEL_HEADER_HEIGHT: f32 = 42.0;
pub(crate) const MAIN_AREA_HEIGHT: f32 = 560.0;

pub(crate) const GRAPH_PANEL_MIN_WIDTH: f32 = 310.0;
pub(crate) const GRAPH_PANEL_MAX_WIDTH: f32 = 420.0;
pub(crate) const GRAPH_PANEL_WIDTH_RATIO: f32 = 0.34;

pub(crate) const COUNTER_TILE_MIN_WIDTH: f32 = 130.0;
pub(crate) const COUNTER_TILE_MAX_WIDTH: f32 = 210.0;
pub(crate) const COUNTER_COLUMN_OPTIONS: [usize; 4] = [6, 3, 2, 1];
pub(crate) const COUNTER_GRID_SPACING: Vec2 = Vec2::new(10.0, 8.0);

pub(crate) const COMPACT_BREAKPOINT: f32 = 720.0;
pub(crate) const SPLIT_BREAKPOINT: f32 = 980.0;

pub(crate) const SOURCES_HEIGHT_MIN: f32 = 88.0;
pub(crate) const SOURCES_HEIGHT_MAX: f32 = 180.0;
pub(crate) const SOURCES_HEIGHT_RATIO: f32 = 0.15;
pub(crate) const NODE_TABLE_MIN_WIDTH: f32 = 230.0;

pub(crate) const TRENDS_COMPACT_VISIBLE_COUNT: usize = 3;
pub(crate) const TRENDS_WIDE_VISIBLE_COUNT: usize = 6;
pub(crate) const TRENDS_COMPACT_NAME_CHARS: usize = 32;
pub(crate) const TRENDS_WIDE_NAME_CHARS: usize = 44;
pub(crate) const TRENDS_ITEM_GAP: f32 = 6.0;
pub(crate) const MIN_TREND_HISTORY_POINTS: usize = 2;

pub(crate) const SPARKLINE_HEIGHT_MIN: f32 = 36.0;
pub(crate) const SPARKLINE_HEIGHT_MAX: f32 = 62.0;
pub(crate) const SPARKLINE_HEIGHT_RATIO: f32 = 0.11;
pub(crate) const SPARKLINE_RADIUS: u8 = 5;
pub(crate) const SPARKLINE_PADDING_X: f32 = 8.0;
pub(crate) const SPARKLINE_PADDING_Y: f32 = 6.0;
pub(crate) const SPARKLINE_BORDER_WIDTH: f32 = 1.0;
pub(crate) const SPARKLINE_STROKE_WIDTH: f32 = 1.8;
pub(crate) const SPARKLINE_BACKGROUND: Color32 = Color32::from_rgb(18, 23, 31);
pub(crate) const SPARKLINE_BORDER: Color32 = Color32::from_rgb(45, 55, 70);
pub(crate) const SPARKLINE_LINE: Color32 = Color32::from_rgb(93, 191, 255);

pub(crate) const METRICS_TABLE_HEIGHT_MIN: f32 = 220.0;
pub(crate) const METRICS_TABLE_HEIGHT_MAX: f32 = 390.0;
pub(crate) const METRICS_TABLE_HEIGHT_RATIO: f32 = 0.38;
pub(crate) const METRICS_TABLE_MIN_WIDTH: f32 = 460.0;
pub(crate) const METRICS_WIDE_SCROLL_MIN_WIDTH: f32 = 1020.0;
pub(crate) const METRICS_WIDE_MIN_COL_WIDTH: f32 = 80.0;
pub(crate) const METRICS_COMPACT_VALUE_WIDTH_RATIO: f32 = 0.28;
pub(crate) const METRICS_COMPACT_VALUE_WIDTH_MIN: f32 = 72.0;
pub(crate) const METRICS_COMPACT_VALUE_WIDTH_MAX: f32 = 140.0;
pub(crate) const METRICS_COMPACT_NAME_MIN_WIDTH: f32 = 72.0;
pub(crate) const METRICS_COMPACT_ROW_HEIGHT: f32 = 22.0;
pub(crate) const METRICS_COMPACT_NAME_CHAR_WIDTH: f32 = 8.0;
pub(crate) const METRICS_COMPACT_NAME_CHARS_MIN: usize = 8;
pub(crate) const METRICS_COMPACT_NAME_CHARS_MAX: usize = 64;
pub(crate) const METRICS_COMPACT_ROW_AREA_MIN_HEIGHT: f32 = 80.0;
pub(crate) const METRICS_COMPACT_ROW_EVEN: Color32 = Color32::from_rgb(18, 22, 29);
pub(crate) const METRICS_COMPACT_ROW_ODD: Color32 = Color32::from_rgb(14, 17, 23);

pub(crate) const EVENT_LOG_HEIGHT_MIN: f32 = 120.0;

pub(crate) const ALERT_CPU_USAGE_WARNING_THRESHOLD: f64 = 90.0;
pub(crate) const ALERT_MEMORY_USAGE_WARNING_THRESHOLD: f64 = 90.0;
pub(crate) const ALERT_ROW_WARNING: Color32 = Color32::from_rgb(45, 37, 22);
pub(crate) const ALERT_ROW_CRITICAL: Color32 = Color32::from_rgb(48, 25, 30);
pub(crate) const ALERT_CLEAR_COLOR: Color32 = Color32::from_rgb(76, 175, 112);
pub(crate) const ALERT_WARNING_COLOR: Color32 = Color32::from_rgb(210, 168, 74);
pub(crate) const ALERT_CRITICAL_COLOR: Color32 = Color32::from_rgb(235, 92, 92);
pub(crate) const ALERT_BADGE_BACKGROUND: Color32 = Color32::from_rgb(25, 31, 42);
pub(crate) const ALERT_BADGE_BORDER_WIDTH: f32 = 1.0;
pub(crate) const ALERT_BADGE_RADIUS: u8 = 6;
pub(crate) const ALERT_BADGE_MARGIN_X: i8 = 10;
pub(crate) const ALERT_BADGE_MARGIN_Y: i8 = 5;

pub(crate) const PAGE_BACKGROUND: Color32 = Color32::from_rgb(15, 18, 24);
pub(crate) const TITLE_COLOR: Color32 = Color32::from_rgb(231, 238, 247);
pub(crate) const MUTED_TEXT_COLOR: Color32 = Color32::from_rgb(144, 155, 172);
pub(crate) const PLACEHOLDER_TEXT_COLOR: Color32 = Color32::from_gray(130);
pub(crate) const TABLE_HEADER_COLOR: Color32 = Color32::from_rgb(166, 188, 218);

pub(crate) const APP_TITLE_SIZE: f32 = 24.0;
pub(crate) const APP_SUBTITLE_SIZE: f32 = 13.0;

pub(crate) const STATUS_STARTING_COLOR: Color32 = Color32::from_rgb(210, 168, 74);
pub(crate) const STATUS_LISTENING_COLOR: Color32 = Color32::from_rgb(76, 175, 112);
pub(crate) const STATUS_ERROR_COLOR: Color32 = Color32::from_rgb(210, 83, 83);
pub(crate) const STATUS_BADGE_BACKGROUND: Color32 = Color32::from_rgb(25, 31, 42);
pub(crate) const STATUS_BADGE_BORDER_WIDTH: f32 = 1.0;
pub(crate) const STATUS_BADGE_RADIUS: u8 = 6;
pub(crate) const STATUS_BADGE_MARGIN_X: i8 = 10;
pub(crate) const STATUS_BADGE_MARGIN_Y: i8 = 5;

pub(crate) const STAT_TILE_MARGIN: i8 = 10;
pub(crate) const STAT_TILE_CONTENT_HEIGHT: f32 = 24.0;
pub(crate) const STAT_TILE_LABEL_GAP: f32 = 8.0;
pub(crate) const STAT_TILE_NUMBER_CHAR_WIDTH: f32 = 12.0;
pub(crate) const STAT_TILE_NUMBER_MIN_WIDTH: f32 = 18.0;
pub(crate) const STAT_TILE_NUMBER_MAX_WIDTH: f32 = 72.0;
pub(crate) const STAT_TILE_NUMBER_MAX_RATIO: f32 = 0.52;
pub(crate) const STAT_TILE_BACKGROUND: Color32 = Color32::from_rgb(23, 28, 38);
pub(crate) const STAT_TILE_BORDER: Color32 = Color32::from_rgb(50, 58, 72);
pub(crate) const STAT_TILE_BORDER_WIDTH: f32 = 1.0;
pub(crate) const STAT_TILE_RADIUS: u8 = 6;
pub(crate) const STAT_TILE_NUMBER_SIZE: f32 = 20.0;
pub(crate) const STAT_TILE_LABEL_SIZE: f32 = 12.0;
pub(crate) const STAT_TILE_LABEL_COLOR: Color32 = Color32::from_gray(140);

pub(crate) const EVENT_METRICS_COLOR: Color32 = Color32::from_rgb(116, 190, 255);
pub(crate) const EVENT_TRACES_COLOR: Color32 = Color32::from_rgb(193, 145, 255);
pub(crate) const EVENT_LOGS_COLOR: Color32 = Color32::from_rgb(244, 178, 93);
pub(crate) const EVENT_ERROR_COLOR: Color32 = Color32::from_rgb(235, 92, 92);
pub(crate) const EVENT_EXTENSION_COLOR: Color32 = Color32::from_rgb(210, 168, 74);
pub(crate) const EVENT_ALERT_COLOR: Color32 = Color32::from_rgb(235, 92, 92);
pub(crate) const EVENT_RESOLVED_COLOR: Color32 = Color32::from_rgb(76, 175, 112);
pub(crate) const EVENT_DEFAULT_COLOR: Color32 = Color32::from_gray(150);

pub(crate) const METRIC_RESOURCE_SOURCE_KEY: &str = "skid_monitor.source";
pub(crate) const METRIC_SERVICE_NAME_KEY: &str = "service.name";
pub(crate) const METRIC_UNKNOWN_SOURCE: &str = "unknown";
pub(crate) const METRIC_EMPTY_FIELD: &str = "-";
pub(crate) const METRIC_ATTR_PREVIEW_COUNT: usize = 4;
pub(crate) const METRIC_TREND_ATTR_COUNT: usize = 3;
pub(crate) const METRIC_BYTE_UNIT: &str = "By";
pub(crate) const BYTE_UNIT_BASE: f64 = 1024.0;
pub(crate) const BYTE_DISPLAY_UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
