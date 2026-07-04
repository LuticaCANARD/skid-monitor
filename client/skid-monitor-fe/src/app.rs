use crate::components::{
    counters, event_log, header,
    layout::{
        centered_content, graph_panel_width, remaining_height, ContentLayout, LayoutMode,
        PanelLimits,
    },
    metrics, sources, trends,
};
use crate::config;
use crate::model::{EventRow, MetricSample, ReceiverMessage, SignalCounters, Status};
use crate::signal::{metric_samples, spawn_receiver};
use crate::utils::push_capped;
use eframe::egui;
use skid_protocol::protocol::Signal;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

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

    fn main_stack(&self, ui: &mut egui::Ui, compact: bool, panel_width: f32, limits: PanelLimits) {
        sources::show(ui, &self.source_counts, panel_width, limits.sources_height);
        ui.add_space(config::SECTION_GAP);
        trends::show(
            ui,
            compact,
            panel_width,
            limits.trends_height,
            &self.metrics,
            &self.metric_history,
        );
        ui.add_space(config::SECTION_GAP);
        metrics::show(
            ui,
            compact,
            panel_width,
            limits.metrics_height,
            &self.metrics,
        );
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
                sources::show(ui, &self.source_counts, graph_width, limits.sources_height);
                ui.add_space(config::SECTION_GAP);
                trends::show(
                    ui,
                    compact,
                    graph_width,
                    limits.trends_height,
                    &self.metrics,
                    &self.metric_history,
                );
            });
            ui.vertical(|ui| {
                ui.set_width(metrics_width);
                metrics::show(
                    ui,
                    compact,
                    metrics_width,
                    limits.metrics_height,
                    &self.metrics,
                );
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
                    .show_viewport(ui, |ui, _viewport| {
                        let content = ContentLayout::for_viewport(ui.clip_rect().size());

                        centered_content(ui, content, |ui| {
                            let panel_width = ui.available_width();
                            let layout = LayoutMode::for_width(panel_width);
                            let compact = layout.is_compact();

                            header::show(ui, compact, &self.status, self.started_at.elapsed());
                            ui.add_space(config::HEADER_COUNTER_GAP);
                            counters::show(ui, &self.counters);
                            ui.add_space(config::SECTION_GAP);

                            let limits = PanelLimits::for_remaining_height(
                                remaining_height(ui, content),
                                layout,
                            );
                            match layout {
                                LayoutMode::Split => {
                                    self.main_split(ui, compact, panel_width, limits);
                                }
                                LayoutMode::Stacked | LayoutMode::Compact => {
                                    self.main_stack(ui, compact, panel_width, limits);
                                }
                            }
                            ui.add_space(config::SECTION_GAP);
                            event_log::show(ui, panel_width, limits.event_log_height, &self.events);
                            ui.add_space(content.bottom_margin);
                        });
                    });
            });
    }
}
