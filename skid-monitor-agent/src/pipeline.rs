//! Configured signal pipeline runtime.

use crate::config::{AgentConfig, PipelineConfig, ProcessorConfig};
use crate::exporters::{self, RequiredExporterFailure};
use skid_protocol::protocol::Signal;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    SelfObservation,
    Device,
    Otlp,
    DatabaseLogs,
}

#[derive(Debug, Clone)]
pub struct SignalPipeline {
    inner: Arc<PipelineRuntime>,
}

#[derive(Debug, Clone)]
struct PipelineRuntime {
    processors: BTreeMap<String, ProcessorConfig>,
    pipelines: PipelineRoutes,
    exporters: exporters::SignalExporters,
}

#[derive(Debug, Clone)]
struct PipelineRoutes {
    metrics: PipelineConfig,
    traces: PipelineConfig,
    logs: PipelineConfig,
}

impl SignalPipeline {
    pub fn from_config(config: &AgentConfig) -> Result<Self, String> {
        Ok(Self {
            inner: Arc::new(PipelineRuntime {
                processors: config.processors.clone(),
                pipelines: PipelineRoutes {
                    metrics: config.pipelines.metrics.clone(),
                    traces: config.pipelines.traces.clone(),
                    logs: config.pipelines.logs.clone(),
                },
                exporters: exporters::SignalExporters::from_config(config)?,
            }),
        })
    }

    pub async fn export(
        &self,
        receiver: ReceiverKind,
        mut signal: Signal,
    ) -> Result<(), PipelineExportError> {
        let signal_kind = signal.kind();
        let pipeline = self.pipeline_for(&signal);
        if !pipeline
            .receivers
            .iter()
            .any(|candidate| candidate == receiver.as_str())
        {
            debug!(
                receiver = receiver.as_str(),
                signal = signal.kind(),
                "signal skipped by pipeline receiver filter"
            );
            return Ok(());
        }

        for processor_name in &pipeline.processors {
            self.apply_processor(processor_name, &mut signal);
        }

        // All listed exporters are currently required. Always attempt every
        // exporter so one failure does not hide the status of the others.
        let mut failures = Vec::new();
        for exporter_name in &pipeline.exporters {
            if let Err(failure) = self.inner.exporters.export(exporter_name, &signal).await {
                failures.push(failure);
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(PipelineExportError {
                receiver,
                signal_kind,
                failures,
            })
        }
    }

    fn pipeline_for(&self, signal: &Signal) -> &PipelineConfig {
        match signal {
            Signal::Metrics(_) => &self.inner.pipelines.metrics,
            Signal::Traces(_) => &self.inner.pipelines.traces,
            Signal::Logs(_) => &self.inner.pipelines.logs,
        }
    }

    fn apply_processor(&self, name: &str, _signal: &mut Signal) {
        match self.inner.processors.get(name) {
            Some(ProcessorConfig::Batch) => {
                // The batch processor is a named pipeline slot for now. The current
                // cycle already emits one OTLP request per signal type.
            }
            None => warn!(processor = name, "pipeline references missing processor"),
        }
    }
}

/// Aggregate failure for the required exporters of one pipeline signal.
#[derive(Debug)]
pub struct PipelineExportError {
    receiver: ReceiverKind,
    signal_kind: &'static str,
    failures: Vec<RequiredExporterFailure>,
}

impl PipelineExportError {
    pub fn receiver(&self) -> ReceiverKind {
        self.receiver
    }

    pub fn signal_kind(&self) -> &'static str {
        self.signal_kind
    }

    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }
}

impl Display for PipelineExportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} required downstream exporter(s) failed for {} from {}",
            self.failures.len(),
            self.signal_kind,
            self.receiver.as_str()
        )?;
        for failure in &self.failures {
            write!(
                formatter,
                "; {:?}: {}",
                failure.exporter(),
                failure.message()
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for PipelineExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.failures
            .first()
            .map(|failure| failure as &(dyn std::error::Error + 'static))
    }
}

impl ReceiverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReceiverKind::SelfObservation => "self_observation",
            ReceiverKind::Device => "device",
            ReceiverKind::Otlp => "otlp",
            ReceiverKind::DatabaseLogs => "database_logs",
        }
    }
}

trait SignalKind {
    fn kind(&self) -> &'static str;
}

impl SignalKind for Signal {
    fn kind(&self) -> &'static str {
        match self {
            Signal::Metrics(_) => "metrics",
            Signal::Traces(_) => "traces",
            Signal::Logs(_) => "logs",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skid_protocol::otlp::ExportMetricsServiceRequest;

    #[tokio::test]
    async fn required_exporter_failures_are_all_aggregated() {
        let config: AgentConfig = serde_json::from_str(
            r#"
            {
              "exporters": {
                "debug": { "type": "logging" }
              },
              "pipelines": {
                "metrics": {
                  "receivers": ["self_observation"],
                  "exporters": ["missing-a", "debug", "missing-b"]
                },
                "traces": { "exporters": ["debug"] },
                "logs": { "exporters": ["debug"] }
              }
            }
            "#,
        )
        .unwrap();
        let pipeline = SignalPipeline::from_config(&config).unwrap();

        let error = pipeline
            .export(
                ReceiverKind::SelfObservation,
                Signal::Metrics(ExportMetricsServiceRequest::default()),
            )
            .await
            .unwrap_err();

        assert_eq!(error.receiver(), ReceiverKind::SelfObservation);
        assert_eq!(error.signal_kind(), "metrics");
        assert_eq!(error.failure_count(), 2);
        let display = error.to_string();
        assert!(display.contains("missing-a"));
        assert!(display.contains("missing-b"));
    }
}
