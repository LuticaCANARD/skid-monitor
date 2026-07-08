//! Concrete exporter implementations.

use crate::config::{AgentConfig, ExporterConfig};
use crate::transport;
use skid_protocol::otlp::tonic::collector::logs::v1::logs_service_client::LogsServiceClient;
use skid_protocol::otlp::tonic::collector::metrics::v1::metrics_service_client::MetricsServiceClient;
use skid_protocol::otlp::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
use skid_protocol::protocol::Signal;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct SignalExporters {
    inner: Arc<BTreeMap<String, ExporterConfig>>,
}

impl SignalExporters {
    pub fn from_config(config: &AgentConfig) -> Self {
        Self {
            inner: Arc::new(config.exporters.clone()),
        }
    }

    pub async fn export(&self, name: &str, signal: &Signal) {
        match self.inner.get(name) {
            Some(exporter) => {
                if let Err(err) = export_one(name, exporter, signal).await {
                    warn!(
                        exporter = name,
                        signal = signal.kind(),
                        %err,
                        "signal export failed"
                    );
                }
            }
            None => warn!(
                exporter = name,
                signal = signal.kind(),
                "pipeline references missing exporter"
            ),
        }
    }
}

async fn export_one(name: &str, exporter: &ExporterConfig, signal: &Signal) -> Result<(), String> {
    match exporter {
        ExporterConfig::SkidClient { addr } => {
            let resolved_addr = resolve_client_addr(addr.as_ref());
            transport::send_to_client(signal, resolved_addr.as_deref())
        }
        ExporterConfig::Logging { include_json } => {
            if *include_json {
                let json = serde_json::to_string(signal)
                    .map_err(|err| format!("serialize signal for logging exporter: {err}"))?;
                info!(
                    exporter = name,
                    signal = signal.kind(),
                    count = signal.item_count(),
                    %json,
                    "signal exported to log"
                );
            } else {
                info!(
                    exporter = name,
                    signal = signal.kind(),
                    count = signal.item_count(),
                    "signal exported to log"
                );
            }
            Ok(())
        }
        ExporterConfig::Otlp { endpoint } => export_otlp(endpoint, signal).await,
    }
}

async fn export_otlp(endpoint: &str, signal: &Signal) -> Result<(), String> {
    let endpoint = normalize_endpoint(endpoint);
    match signal {
        Signal::Metrics(request) => {
            let mut client = MetricsServiceClient::connect(endpoint)
                .await
                .map_err(|err| format!("connect OTLP metrics exporter: {err}"))?;
            client
                .export(request.clone())
                .await
                .map_err(|err| format!("export OTLP metrics: {err}"))?;
        }
        Signal::Traces(request) => {
            let mut client = TraceServiceClient::connect(endpoint)
                .await
                .map_err(|err| format!("connect OTLP trace exporter: {err}"))?;
            client
                .export(request.clone())
                .await
                .map_err(|err| format!("export OTLP traces: {err}"))?;
        }
        Signal::Logs(request) => {
            let mut client = LogsServiceClient::connect(endpoint)
                .await
                .map_err(|err| format!("connect OTLP logs exporter: {err}"))?;
            client
                .export(request.clone())
                .await
                .map_err(|err| format!("export OTLP logs: {err}"))?;
        }
    }
    Ok(())
}

fn resolve_client_addr(configured: Option<&String>) -> Option<String> {
    configured
        .filter(|addr| !addr.trim().is_empty())
        .cloned()
        .or_else(|| env_or_legacy("SKID_MONITOR_CLIENT_ADDR", "MONITOR_CAT_CLIENT_ADDR").ok())
        .filter(|addr| !addr.trim().is_empty())
}

fn normalize_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    }
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

trait SignalExt {
    fn kind(&self) -> &'static str;
    fn item_count(&self) -> usize;
}

impl SignalExt for Signal {
    fn kind(&self) -> &'static str {
        match self {
            Signal::Metrics(_) => "metrics",
            Signal::Traces(_) => "traces",
            Signal::Logs(_) => "logs",
        }
    }

    fn item_count(&self) -> usize {
        match self {
            Signal::Metrics(request) => request
                .resource_metrics
                .iter()
                .flat_map(|rm| &rm.scope_metrics)
                .map(|sm| sm.metrics.len())
                .sum(),
            Signal::Traces(request) => request
                .resource_spans
                .iter()
                .flat_map(|rs| &rs.scope_spans)
                .map(|ss| ss.spans.len())
                .sum(),
            Signal::Logs(request) => request
                .resource_logs
                .iter()
                .flat_map(|rl| &rl.scope_logs)
                .map(|sl| sl.log_records.len())
                .sum(),
        }
    }
}
