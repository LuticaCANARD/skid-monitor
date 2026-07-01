//! skid-monitor agent.
//!
//! 호스트/인프라에서 정보를 수집(OpenTelemetry, k8s 인프라 시스템 등)하여
//! `skid_protocol` 프로토콜로 client에 전송하는 수집 agent.
//!
//! 현재는 server 자신을 OpenTelemetry SDK로 자체 계측하여 metrics/traces/logs를 생성하고,
//! device/OTLP receiver 신호와 함께 agent pipeline으로 내보낸다.

mod collector;
mod config;
mod device_socket;
mod exporters;
mod otlp_receiver;
mod pipeline;
mod system_metrics;
mod telemetry;
mod transport;

use pipeline::{ReceiverKind, SignalPipeline};
use skid_protocol::protocol::Signal;
use skid_protocol::{
    metrics::export_metrics,
    otlp::{ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest},
};
use std::time::Duration;
use tracing::{info, instrument, warn};

#[tokio::main]
async fn main() {
    let config = match config::AgentConfig::load() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("skid-monitor-agent config error: {err}");
            std::process::exit(2);
        }
    };
    let guard = telemetry::init();
    let pipeline = SignalPipeline::from_config(&config);
    info!("skid-monitor agent starting...");

    if config.receivers.device.enabled {
        let addr = config.receivers.device.listen_addr.clone();
        let pipeline = pipeline.clone();
        tokio::spawn(async move {
            if let Err(err) = device_socket::serve(addr, pipeline).await {
                warn!(%err, "observation device socket stopped");
            }
        });
    }

    if config.receivers.otlp.enabled {
        let addr = config.receivers.otlp.grpc_addr.clone();
        let pipeline = pipeline.clone();
        tokio::spawn(async move {
            if let Err(err) = otlp_receiver::serve(addr, pipeline).await {
                warn!(%err, "OTLP gRPC receiver stopped");
            }
        });
    }

    let mut system_sampler = system_metrics::SystemSampler::new();
    if config.receivers.self_observation.enabled {
        let cycle_interval = Duration::from_secs(config.receivers.self_observation.interval_secs);
        let mut interval = tokio::time::interval(cycle_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => run_cycle(&guard, &mut system_sampler, &pipeline).await,
                _ = tokio::signal::ctrl_c() => {
                    info!("shutdown signal received");
                    break;
                }
            }
        }
    } else if tokio::signal::ctrl_c().await.is_ok() {
        info!("shutdown signal received");
    }

    // 배치 워커에 남은 신호를 flush 한다.
    guard.shutdown();
}

/// 한 주기: 자체 계측한 세 신호를 읽어 pipeline으로 보낸다.
///
/// `#[instrument]`로 이 함수 자체가 OTel span이 되고, 그 span은 다음 주기의 trace 수집에 잡힌다
/// (self-observation).
#[instrument(skip(guard, system_sampler, pipeline))]
async fn run_cycle(
    guard: &telemetry::TelemetryGuard,
    system_sampler: &mut system_metrics::SystemSampler,
    pipeline: &SignalPipeline,
) {
    let mut metrics = collector::collect(guard);
    let system_metrics = export_metrics(
        system_sampler.collect(),
        "skid-monitor-agent",
        "skid-monitor-system",
    );
    metrics
        .resource_metrics
        .extend(system_metrics.resource_metrics);
    info!(count = metric_count(&metrics), "collected metrics");
    pipeline
        .export(ReceiverKind::SelfObservation, Signal::Metrics(metrics))
        .await;

    let spans = collector::collect_spans(guard);
    info!(count = span_count(&spans), "collected spans");
    pipeline
        .export(ReceiverKind::SelfObservation, Signal::Traces(spans))
        .await;

    let logs = collector::collect_logs(guard);
    info!(count = log_count(&logs), "collected logs");
    pipeline
        .export(ReceiverKind::SelfObservation, Signal::Logs(logs))
        .await;
}

fn metric_count(request: &ExportMetricsServiceRequest) -> usize {
    request
        .resource_metrics
        .iter()
        .flat_map(|rm| &rm.scope_metrics)
        .map(|sm| sm.metrics.len())
        .sum()
}

fn span_count(request: &ExportTraceServiceRequest) -> usize {
    request
        .resource_spans
        .iter()
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum()
}

fn log_count(request: &ExportLogsServiceRequest) -> usize {
    request
        .resource_logs
        .iter()
        .flat_map(|rl| &rl.scope_logs)
        .map(|sl| sl.log_records.len())
        .sum()
}
