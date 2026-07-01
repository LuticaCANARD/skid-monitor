//! Configured signal pipeline runtime.

use crate::config::{AgentConfig, PipelineConfig, ProcessorConfig};
use crate::exporters;
use skid_protocol::protocol::Signal;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverKind {
    SelfObservation,
    Device,
    Otlp,
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
    pub fn from_config(config: &AgentConfig) -> Self {
        Self {
            inner: Arc::new(PipelineRuntime {
                processors: config.processors.clone(),
                pipelines: PipelineRoutes {
                    metrics: config.pipelines.metrics.clone(),
                    traces: config.pipelines.traces.clone(),
                    logs: config.pipelines.logs.clone(),
                },
                exporters: exporters::SignalExporters::from_config(config),
            }),
        }
    }

    pub async fn export(&self, receiver: ReceiverKind, mut signal: Signal) {
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
            return;
        }

        for processor_name in &pipeline.processors {
            self.apply_processor(processor_name, &mut signal);
        }

        for exporter_name in &pipeline.exporters {
            self.inner.exporters.export(exporter_name, &signal).await;
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

impl ReceiverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReceiverKind::SelfObservation => "self_observation",
            ReceiverKind::Device => "device",
            ReceiverKind::Otlp => "otlp",
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
