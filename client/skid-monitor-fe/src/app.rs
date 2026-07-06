use crate::alert::AlertStore;
use crate::components::{
    counters, event_log, header,
    layout::{
        ContentLayout, LayoutMode, PanelLimits, centered_content, graph_panel_width,
        remaining_height,
    },
    metrics, sources, trends,
};
use crate::config;
use crate::model::{
    AlertChange, AlertSeverity, AlertStatus, AlertTransition, EventRow, MetricSample,
    ReceiverMessage, SignalCounters, Status,
};
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
    alerts: AlertStore,
}

#[derive(Clone, Copy)]
enum MainPanel {
    Sources,
    Trends,
    Metrics,
}

trait PanelTemplate {
    fn height(self, limits: PanelLimits) -> f32;

    fn render(
        self,
        app: &ControlRoomApp,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        panel_height: f32,
    );
}

impl PanelTemplate for MainPanel {
    fn height(self, limits: PanelLimits) -> f32 {
        match self {
            Self::Sources => limits.sources_height,
            Self::Trends => limits.trends_height,
            Self::Metrics => limits.metrics_height,
        }
    }

    fn render(
        self,
        app: &ControlRoomApp,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        panel_height: f32,
    ) {
        match self {
            Self::Sources => sources::show(ui, &app.source_counts, panel_width, panel_height),
            Self::Trends => trends::show(
                ui,
                compact,
                panel_width,
                panel_height,
                &app.metrics,
                &app.metric_history,
            ),
            Self::Metrics => metrics::show(
                ui,
                compact,
                panel_width,
                panel_height,
                &app.metrics,
                &app.alerts,
            ),
        }
    }
}

const STACKED_MAIN_PANELS: [MainPanel; 3] =
    [MainPanel::Sources, MainPanel::Trends, MainPanel::Metrics];
const GRAPH_MAIN_PANELS: [MainPanel; 2] = [MainPanel::Sources, MainPanel::Trends];

fn severity_label(severity: AlertSeverity) -> &'static str {
    match severity {
        AlertSeverity::Warning => "warning",
        AlertSeverity::Critical => "critical",
    }
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
            alerts: AlertStore::default(),
        }
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                ReceiverMessage::Listening(addr) => {
                    self.status = Status::Listening(addr.clone());
                    self.push_event("receiver", format!("listening on {addr}"));
                    let change = self
                        .alerts
                        .observe_receiver_recovered("receiver is listening");
                    self.push_alert_change(change);
                }
                ReceiverMessage::Signal(signal) => {
                    let change = self
                        .alerts
                        .observe_receiver_recovered("receiver received a signal");
                    self.push_alert_change(change);
                    self.ingest_signal(signal);
                }
                ReceiverMessage::Error(error) => {
                    self.status = Status::Error(error.clone());
                    self.push_event("error", error);
                    let change = match &self.status {
                        Status::Error(error) => self.alerts.observe_receiver_error(error),
                        Status::Starting | Status::Listening(_) => None,
                    };
                    self.push_alert_change(change);
                }
                ReceiverMessage::ExtensionError(error) => {
                    self.push_event("extension", error.clone());
                    let change = self.alerts.observe_extension_error(&error);
                    self.push_alert_change(change);
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
                    let change = self.alerts.observe_metric(&sample);
                    self.push_alert_change(change);
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

    fn push_alert_change(&mut self, change: Option<AlertChange>) {
        let Some(change) = change else {
            return;
        };
        let status = match change.snapshot.status {
            AlertStatus::Firing => "firing",
            AlertStatus::Resolved => "resolved",
        };
        let kind = match change.transition {
            AlertTransition::Fired => "alert",
            AlertTransition::Resolved => "resolved",
        };
        let severity = severity_label(change.snapshot.severity);

        self.push_event(
            kind,
            format!(
                "{status} {severity} {} [{}] from {}: {}",
                change.snapshot.summary,
                change.snapshot.rule_id,
                change.snapshot.source,
                change.snapshot.detail
            ),
        );
    }

    fn main_stack(&self, ui: &mut egui::Ui, compact: bool, panel_width: f32, limits: PanelLimits) {
        self.panel_stack(ui, compact, panel_width, limits, &STACKED_MAIN_PANELS);
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
                self.panel_stack(ui, compact, graph_width, limits, &GRAPH_MAIN_PANELS);
            });
            ui.vertical(|ui| {
                ui.set_width(metrics_width);
                self.panel(ui, compact, metrics_width, limits, MainPanel::Metrics);
            });
        });
    }

    fn panel_stack(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
        panels: &[MainPanel],
    ) {
        for (index, panel) in panels.iter().copied().enumerate() {
            if index > 0 {
                ui.add_space(config::SECTION_GAP);
            }
            self.panel(ui, compact, panel_width, limits, panel);
        }
    }

    fn panel(
        &self,
        ui: &mut egui::Ui,
        compact: bool,
        panel_width: f32,
        limits: PanelLimits,
        panel: MainPanel,
    ) {
        panel.render(self, ui, compact, panel_width, panel.height(limits));
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

                            header::show(
                                ui,
                                compact,
                                &self.status,
                                self.alerts.summary(),
                                self.started_at.elapsed(),
                            );
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
