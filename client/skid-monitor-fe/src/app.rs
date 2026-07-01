use crate::components::{draw_sparkline, kind_color, stat_tile, status_badge, table_header};
use crate::config;
use crate::model::{EventRow, MetricSample, ReceiverMessage, SignalCounters, Status};
use crate::signal::{metric_samples, spawn_receiver};
use crate::utils::{format_duration, format_f64, push_capped, shorten};
use eframe::egui::{self, RichText, Vec2};
use skid_protocol::protocol::Signal;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum LayoutMode {
    Compact,
    Stacked,
    Split,
}

impl LayoutMode {
    fn for_width(width: f32) -> Self {
        if width < config::COMPACT_BREAKPOINT {
            Self::Compact
        } else if width < config::SPLIT_BREAKPOINT {
            Self::Stacked
        } else {
            Self::Split
        }
    }

    fn is_compact(self) -> bool {
        matches!(self, Self::Compact)
    }
}

#[derive(Clone, Copy)]
struct PanelLimits {
    sources_height: f32,
    trends_height: f32,
    metrics_height: f32,
    event_log_height: f32,
}

impl PanelLimits {
    fn for_viewport(height: f32, layout: LayoutMode) -> Self {
        let trends_ratio = match layout {
            LayoutMode::Compact => config::TRENDS_COMPACT_HEIGHT_RATIO,
            LayoutMode::Stacked => config::TRENDS_STACKED_HEIGHT_RATIO,
            LayoutMode::Split => config::TRENDS_SPLIT_HEIGHT_RATIO,
        };

        Self {
            sources_height: clamped_extent(
                height,
                config::SOURCES_HEIGHT_RATIO,
                config::SOURCES_HEIGHT_MIN,
                config::SOURCES_HEIGHT_MAX,
            ),
            trends_height: clamped_extent(
                height,
                trends_ratio,
                config::TRENDS_HEIGHT_MIN,
                config::TRENDS_HEIGHT_MAX,
            ),
            metrics_height: clamped_extent(
                height,
                config::METRICS_TABLE_HEIGHT_RATIO,
                config::METRICS_TABLE_HEIGHT_MIN,
                config::METRICS_TABLE_HEIGHT_MAX,
            ),
            event_log_height: clamped_extent(
                height,
                config::EVENT_LOG_HEIGHT_RATIO,
                config::EVENT_LOG_HEIGHT_MIN,
                config::EVENT_LOG_HEIGHT_MAX,
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct ContentLayout {
    width: f32,
    side_margin: f32,
    bottom_margin: f32,
}

impl ContentLayout {
    fn for_viewport(size: Vec2) -> Self {
        let readable_margin = clamped_extent(
            size.x,
            config::CONTENT_SIDE_MARGIN_RATIO,
            config::CONTENT_SIDE_MARGIN_MIN,
            config::CONTENT_SIDE_MARGIN_MAX,
        );
        let width = (size.x - readable_margin * 2.0)
            .clamp(1.0, config::CONTENT_MAX_WIDTH)
            .min(size.x.max(1.0));
        let side_margin = ((size.x - width) * 0.5).max(0.0);

        Self {
            width,
            side_margin,
            bottom_margin: clamped_extent(
                size.y,
                config::CONTENT_BOTTOM_MARGIN_RATIO,
                config::CONTENT_BOTTOM_MARGIN_MIN,
                config::CONTENT_BOTTOM_MARGIN_MAX,
            ),
        }
    }
}

fn clamped_extent(total: f32, ratio: f32, min: f32, max: f32) -> f32 {
    (total * ratio).clamp(min, max)
}

fn graph_panel_width(content_width: f32) -> f32 {
    (content_width * config::GRAPH_PANEL_WIDTH_RATIO)
        .clamp(config::GRAPH_PANEL_MIN_WIDTH, config::GRAPH_PANEL_MAX_WIDTH)
}

fn sparkline_height(width: f32) -> f32 {
    (width * config::SPARKLINE_HEIGHT_RATIO)
        .clamp(config::SPARKLINE_HEIGHT_MIN, config::SPARKLINE_HEIGHT_MAX)
}

fn counter_columns(available_width: f32, spacing: f32, item_count: usize) -> usize {
    config::COUNTER_COLUMN_OPTIONS
        .into_iter()
        .filter(|columns| *columns <= item_count)
        .find(|columns| {
            let total_spacing = spacing * columns.saturating_sub(1) as f32;
            let required_width = config::COUNTER_TILE_MIN_WIDTH * *columns as f32 + total_spacing;
            available_width >= required_width
        })
        .unwrap_or(1)
}

fn centered_content<R>(
    ui: &mut egui::Ui,
    content: ContentLayout,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let viewport = ui.clip_rect();
    let top = ui.cursor().top();
    let max_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.left() + content.side_margin, top),
        egui::vec2(content.width, viewport.height().max(1.0)),
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt("centered-content")
            .max_rect(max_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(content.width);
            add_contents(ui)
        },
    )
}

pub(crate) struct ControlRoomApp {
    rx: Receiver<ReceiverMessage>,
    started_at: Instant,
    status: Status,
    counters: SignalCounters,
    events: VecDeque<EventRow>,
    metrics: VecDeque<MetricSample>,
    metric_history: BTreeMap<String, VecDeque<f64>>,
    source_counts: BTreeMap<String, usize>,
}

impl ControlRoomApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.global_style_mut(|style| {
            style.spacing.item_spacing = config::GLOBAL_ITEM_SPACING;
            style.spacing.button_padding = config::GLOBAL_BUTTON_PADDING;
        });

        Self {
            rx: spawn_receiver(),
            started_at: Instant::now(),
            status: Status::Starting,
            counters: SignalCounters::default(),
            events: VecDeque::new(),
            metrics: VecDeque::new(),
            metric_history: BTreeMap::new(),
            source_counts: BTreeMap::new(),
        }
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                ReceiverMessage::Listening(addr) => {
                    self.status = Status::Listening(addr.clone());
                    self.push_event("receiver", format!("listening on {addr}"));
                }
                ReceiverMessage::Signal(signal) => self.ingest_signal(signal),
                ReceiverMessage::Error(error) => {
                    self.status = Status::Error(error.clone());
                    self.push_event("error", error);
                }
                ReceiverMessage::ExtensionError(error) => {
                    self.push_event("extension", error);
                }
            }
        }
    }

    fn ingest_signal(&mut self, signal: Signal) {
        match &signal {
            Signal::Metrics(request) => {
                self.counters.metrics += 1;
                let samples = metric_samples(request);
                let sample_count = samples.len();
                self.counters.metric_points += sample_count;
                for sample in samples {
                    *self.source_counts.entry(sample.source.clone()).or_default() += 1;
                    if let Some(value) = sample.numeric {
                        push_capped(
                            self.metric_history
                                .entry(sample.trend_key.clone())
                                .or_default(),
                            value,
                            config::MAX_HISTORY_POINTS,
                        );
                    }
                    push_capped(&mut self.metrics, sample, config::MAX_METRICS);
                }
                self.push_event(
                    "metrics",
                    format!(
                        "received {} metric points from {} resources",
                        sample_count,
                        request.resource_metrics.len()
                    ),
                );
            }
            Signal::Traces(request) => {
                let count = request
                    .resource_spans
                    .iter()
                    .flat_map(|resource| &resource.scope_spans)
                    .map(|scope| scope.spans.len())
                    .sum::<usize>();
                self.counters.traces += 1;
                self.counters.spans += count;
                self.push_event("traces", format!("received {count} spans"));
            }
            Signal::Logs(request) => {
                let count = request
                    .resource_logs
                    .iter()
                    .flat_map(|resource| &resource.scope_logs)
                    .map(|scope| scope.log_records.len())
                    .sum::<usize>();
                self.counters.logs += 1;
                self.counters.log_records += count;
                self.push_event("logs", format!("received {count} log records"));
            }
        }
    }

    fn push_event(&mut self, kind: impl Into<String>, message: impl Into<String>) {
        push_capped(
            &mut self.events,
            EventRow {
                at: Instant::now(),
                kind: kind.into(),
                message: message.into(),
            },
            config::MAX_EVENTS,
        );
    }

    fn header(&self, ui: &mut egui::Ui, compact: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(config::APP_TITLE)
                        .size(config::APP_TITLE_SIZE)
                        .strong()
                        .color(config::TITLE_COLOR),
                );
                ui.label(
                    RichText::new(config::APP_SUBTITLE)
                        .size(config::APP_SUBTITLE_SIZE)
                        .color(config::MUTED_TEXT_COLOR),
                );
            });
            if !compact {
                ui.add_space(config::HEADER_STATUS_GAP);
            }
            status_badge(ui, &self.status);
            if !compact {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "uptime {}",
                            format_duration(self.started_at.elapsed())
                        ))
                        .monospace()
                        .color(config::MUTED_TEXT_COLOR),
                    );
                });
            } else {
                ui.label(
                    RichText::new(format!(
                        "uptime {}",
                        format_duration(self.started_at.elapsed())
                    ))
                    .monospace()
                    .color(config::MUTED_TEXT_COLOR),
                );
            }
        });
    }

    fn counters(&self, ui: &mut egui::Ui) {
        let stats = [
            ("metric batches", self.counters.metrics),
            ("metric points", self.counters.metric_points),
            ("trace batches", self.counters.traces),
            ("spans", self.counters.spans),
            ("log batches", self.counters.logs),
            ("log records", self.counters.log_records),
        ];
        let spacing = ui.spacing().item_spacing.x;
        let columns = counter_columns(ui.available_width(), spacing, stats.len());
        let total_spacing = spacing * columns.saturating_sub(1) as f32;
        let tile_width = ((ui.available_width() - total_spacing) / columns as f32).clamp(
            config::COUNTER_TILE_MIN_WIDTH,
            config::COUNTER_TILE_MAX_WIDTH,
        );
        let grid_width = tile_width * columns as f32 + total_spacing;

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(((ui.available_width() - grid_width) * 0.5).max(0.0));
            egui::Grid::new("counter-grid")
                .num_columns(columns)
                .spacing(config::COUNTER_GRID_SPACING)
                .show(ui, |ui| {
                    for (index, (label, value)) in stats.iter().copied().enumerate() {
                        stat_tile(ui, label, value, tile_width);
                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    }

    fn main_stack(&self, ui: &mut egui::Ui, compact: bool, limits: PanelLimits) {
        self.source_summary(ui, limits.sources_height);
        ui.add_space(config::SECTION_GAP);
        self.trends_panel(ui, compact, limits.trends_height);
        ui.add_space(config::SECTION_GAP);
        self.metrics_table(ui, compact, ui.available_width(), limits.metrics_height);
    }

    fn main_split(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        content_width: f32,
        limits: PanelLimits,
    ) {
        let spacing = ui.spacing().item_spacing.x;
        let graph_width = graph_panel_width(content_width);
        let metrics_width =
            (content_width - graph_width - spacing).max(config::METRICS_TABLE_MIN_WIDTH);

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(graph_width);
                self.source_summary(ui, limits.sources_height);
                ui.add_space(config::SECTION_GAP);
                self.trends_panel(ui, compact, limits.trends_height);
            });
            ui.vertical(|ui| {
                ui.set_width(metrics_width);
                self.metrics_table(ui, compact, metrics_width, limits.metrics_height);
            });
        });
    }

    fn source_summary(&self, ui: &mut egui::Ui, max_height: f32) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.set_min_width(config::SOURCES_MIN_WIDTH);
            ui.heading("Sources");
            ui.separator();
            if self.source_counts.is_empty() {
                ui.label(
                    RichText::new("waiting for metrics").color(config::PLACEHOLDER_TEXT_COLOR),
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("sources-scroll")
                .auto_shrink([false, true])
                .max_height(max_height)
                .show(ui, |ui| {
                    for (source, count) in &self.source_counts {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(source).monospace());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(RichText::new(count.to_string()).strong());
                                },
                            );
                        });
                    }
                });
        });
    }

    fn trends_panel(&self, ui: &mut egui::Ui, compact: bool, max_height: f32) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Trends");
            ui.separator();

            let trend_keys = self.visible_trend_keys(if compact {
                config::TRENDS_COMPACT_VISIBLE_COUNT
            } else {
                config::TRENDS_WIDE_VISIBLE_COUNT
            });
            if trend_keys.is_empty() {
                ui.label(
                    RichText::new("waiting for numeric metrics")
                        .color(config::PLACEHOLDER_TEXT_COLOR),
                );
                return;
            }

            let graph_height = sparkline_height(ui.available_width());

            egui::ScrollArea::vertical()
                .id_salt("trends-scroll")
                .auto_shrink([false, true])
                .max_height(max_height)
                .show(ui, |ui| {
                    for key in trend_keys {
                        if let Some(values) = self.metric_history.get(&key) {
                            let latest = values.back().copied().unwrap_or_default();
                            ui.horizontal(|ui| {
                                let name_chars = if compact {
                                    config::TRENDS_COMPACT_NAME_CHARS
                                } else {
                                    config::TRENDS_WIDE_NAME_CHARS
                                };
                                ui.label(RichText::new(shorten(&key, name_chars)).monospace());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format_f64(latest)).monospace().strong(),
                                        );
                                    },
                                );
                            });
                            draw_sparkline(ui, values, graph_height);
                            ui.add_space(config::TRENDS_ITEM_GAP);
                        }
                    }
                });
        });
    }

    fn visible_trend_keys(&self, max: usize) -> Vec<String> {
        let mut keys = Vec::new();
        for sample in self.metrics.iter().rev() {
            if sample.numeric.is_none() {
                continue;
            }
            if self
                .metric_history
                .get(&sample.trend_key)
                .is_none_or(|values| values.len() < config::MIN_TREND_HISTORY_POINTS)
            {
                continue;
            }
            if !keys.iter().any(|key| key == &sample.trend_key) {
                keys.push(sample.trend_key.clone());
            }
            if keys.len() >= max {
                break;
            }
        }
        keys
    }

    fn metrics_table(&self, ui: &mut egui::Ui, compact: bool, panel_width: f32, max_height: f32) {
        ui.group(|ui| {
            let panel_width = panel_width.max(1.0);
            ui.set_width(panel_width);
            ui.heading("Latest Metrics");
            ui.separator();
            if self.metrics.is_empty() {
                ui.label(
                    RichText::new("no metrics received yet").color(config::PLACEHOLDER_TEXT_COLOR),
                );
                return;
            }

            if compact {
                self.compact_metrics_table(ui, panel_width, max_height);
            } else {
                self.wide_metrics_table(ui, panel_width, max_height);
            }
        });
    }

    fn compact_metrics_table(&self, ui: &mut egui::Ui, panel_width: f32, max_height: f32) {
        let row_width = ui.available_width().min(panel_width).max(1.0);
        let spacing = ui.spacing().item_spacing.x;
        let value_width = (row_width * config::METRICS_COMPACT_VALUE_WIDTH_RATIO).clamp(
            config::METRICS_COMPACT_VALUE_WIDTH_MIN,
            config::METRICS_COMPACT_VALUE_WIDTH_MAX,
        );
        let name_width =
            (row_width - value_width - spacing).max(config::METRICS_COMPACT_NAME_MIN_WIDTH);
        let name_chars = ((name_width / config::METRICS_COMPACT_NAME_CHAR_WIDTH).floor() as usize)
            .clamp(
                config::METRICS_COMPACT_NAME_CHARS_MIN,
                config::METRICS_COMPACT_NAME_CHARS_MAX,
            );
        let row_area_height = (max_height - config::METRICS_COMPACT_ROW_HEIGHT)
            .max(config::METRICS_COMPACT_ROW_AREA_MIN_HEIGHT);

        ui.horizontal(|ui| {
            ui.add_sized(
                [name_width, config::METRICS_COMPACT_ROW_HEIGHT],
                egui::Label::new(
                    RichText::new("metric")
                        .strong()
                        .color(config::TABLE_HEADER_COLOR),
                ),
            );
            ui.add_sized(
                [value_width, config::METRICS_COMPACT_ROW_HEIGHT],
                egui::Label::new(
                    RichText::new("value")
                        .strong()
                        .color(config::TABLE_HEADER_COLOR),
                ),
            );
        });

        egui::ScrollArea::vertical()
            .id_salt("metrics-table-scroll-compact")
            .auto_shrink([false, false])
            .max_width(row_width)
            .max_height(row_area_height)
            .show(ui, |ui| {
                ui.set_width(row_width);
                for (index, sample) in self.metrics.iter().rev().enumerate() {
                    let fill = if index % 2 == 0 {
                        config::METRICS_COMPACT_ROW_EVEN
                    } else {
                        config::METRICS_COMPACT_ROW_ODD
                    };

                    egui::Frame::default().fill(fill).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [name_width, config::METRICS_COMPACT_ROW_HEIGHT],
                                egui::Label::new(
                                    RichText::new(shorten(&sample.name, name_chars)).monospace(),
                                ),
                            );
                            ui.add_sized(
                                [value_width, config::METRICS_COMPACT_ROW_HEIGHT],
                                egui::Label::new(RichText::new(&sample.value).monospace().strong()),
                            );
                        });
                    });
                }
            });
    }

    fn wide_metrics_table(&self, ui: &mut egui::Ui, panel_width: f32, max_height: f32) {
        let table_width = panel_width.max(config::METRICS_WIDE_SCROLL_MIN_WIDTH);

        egui::ScrollArea::both()
            .id_salt("metrics-table-scroll-wide")
            .auto_shrink([false, false])
            .max_width(panel_width)
            .max_height(max_height)
            .show(ui, |ui| {
                ui.set_min_width(table_width);
                egui::Grid::new("metrics-grid-wide")
                    .striped(true)
                    .min_col_width(config::METRICS_WIDE_MIN_COL_WIDTH)
                    .show(ui, |ui| {
                        table_header(ui, "metric");
                        table_header(ui, "value");
                        table_header(ui, "source");
                        table_header(ui, "kind");
                        table_header(ui, "attrs");
                        ui.end_row();

                        for sample in self.metrics.iter().rev() {
                            ui.label(RichText::new(&sample.name).monospace());
                            ui.label(RichText::new(&sample.value).monospace().strong());
                            ui.label(RichText::new(&sample.source).monospace());
                            ui.label(&sample.kind);
                            ui.label(RichText::new(&sample.attributes).small());
                            ui.end_row();
                        }
                    });
            });
    }

    fn event_log(&self, ui: &mut egui::Ui, panel_width: f32, max_height: f32) {
        ui.group(|ui| {
            ui.set_width(panel_width);
            ui.heading("Event Log");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("event-log-scroll")
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .max_height(max_height)
                .show(ui, |ui| {
                    for event in &self.events {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format_duration(event.at.elapsed()))
                                    .monospace()
                                    .color(config::PLACEHOLDER_TEXT_COLOR),
                            );
                            ui.label(
                                RichText::new(&event.kind)
                                    .monospace()
                                    .color(kind_color(&event.kind)),
                            );
                            ui.label(&event.message);
                        });
                    }
                });
        });
    }
}

impl eframe::App for ControlRoomApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_messages();
        ui.ctx()
            .request_repaint_after(Duration::from_millis(config::REPAINT_INTERVAL_MS));

        egui::Frame::default()
            .fill(config::PAGE_BACKGROUND)
            .inner_margin(egui::Margin::same(config::CONTENT_FRAME_MARGIN))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("control-room-page-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content = ContentLayout::for_viewport(ui.clip_rect().size());

                        centered_content(ui, content, |ui| {
                            let layout = LayoutMode::for_width(content.width);
                            let compact = layout.is_compact();
                            let limits = PanelLimits::for_viewport(ui.clip_rect().height(), layout);

                            self.header(ui, compact);
                            ui.add_space(config::HEADER_COUNTER_GAP);
                            self.counters(ui);
                            ui.add_space(config::SECTION_GAP);
                            match layout {
                                LayoutMode::Split => {
                                    self.main_split(ui, compact, content.width, limits);
                                }
                                LayoutMode::Stacked | LayoutMode::Compact => {
                                    self.main_stack(ui, compact, limits);
                                }
                            }
                            ui.add_space(config::SECTION_GAP);
                            self.event_log(ui, content.width, limits.event_log_height);
                            ui.add_space(content.bottom_margin);
                        });
                    });
            });
    }
}
