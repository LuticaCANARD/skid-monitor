//! Agent runtime configuration.
//!
//! `SKID_MONITOR_AGENT_CONFIG` can point at a JSON file with Collector-like
//! receiver/processor/exporter/pipeline sections. When it is absent, the legacy
//! environment-variable behavior is used.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

pub const CONFIG_ENV: &str = "SKID_MONITOR_AGENT_CONFIG";

const DEFAULT_CYCLE_INTERVAL_SECS: u64 = 15;
const DEFAULT_DEVICE_LISTEN_ADDR: &str = "127.0.0.1:9101";
const DEFAULT_OTLP_GRPC_ADDR: &str = "127.0.0.1:4317";

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub receivers: ReceiversConfig,
    pub processors: BTreeMap<String, ProcessorConfig>,
    #[serde(default = "default_exporters")]
    pub exporters: BTreeMap<String, ExporterConfig>,
    #[serde(default = "default_pipelines")]
    pub pipelines: PipelinesConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReceiversConfig {
    pub self_observation: SelfObservationReceiverConfig,
    pub device: DeviceReceiverConfig,
    pub otlp: OtlpReceiverConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelfObservationReceiverConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cycle_interval_secs")]
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceReceiverConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_device_listen_addr")]
    pub listen_addr: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OtlpReceiverConfig {
    #[serde(default = "default_otlp_receiver_enabled")]
    pub enabled: bool,
    #[serde(default = "default_otlp_grpc_addr")]
    pub grpc_addr: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessorConfig {
    Batch,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExporterConfig {
    SkidClient {
        #[serde(default)]
        addr: Option<String>,
    },
    Logging {
        #[serde(default)]
        include_json: bool,
    },
    Otlp {
        endpoint: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelinesConfig {
    pub metrics: PipelineConfig,
    pub traces: PipelineConfig,
    pub logs: PipelineConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    #[serde(default = "default_pipeline_receivers")]
    pub receivers: Vec<String>,
    #[serde(default)]
    pub processors: Vec<String>,
    #[serde(default = "default_pipeline_exporters")]
    pub exporters: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Validate(String),
}

impl AgentConfig {
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = if let Ok(path) = std::env::var(CONFIG_ENV) {
            let path = PathBuf::from(path);
            let source = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
                path: path.clone(),
                source,
            })?;
            serde_json::from_str(&source).map_err(|source| ConfigError::Parse {
                path: path.clone(),
                source,
            })?
        } else {
            Self::default()
        };
        config.apply_runtime_defaults();
        config.validate()?;
        Ok(config)
    }

    fn apply_runtime_defaults(&mut self) {
        if self.receivers.self_observation.interval_secs == 0 {
            self.receivers.self_observation.interval_secs = DEFAULT_CYCLE_INTERVAL_SECS;
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.exporters.is_empty() {
            return Err(ConfigError::Validate(
                "at least one exporter must be configured".to_string(),
            ));
        }

        validate_pipeline("metrics", &self.pipelines.metrics, self)?;
        validate_pipeline("traces", &self.pipelines.traces, self)?;
        validate_pipeline("logs", &self.pipelines.logs, self)?;

        for (name, exporter) in &self.exporters {
            if let ExporterConfig::Otlp { endpoint } = exporter
                && endpoint.trim().is_empty()
            {
                return Err(ConfigError::Validate(format!(
                    "exporter {name:?} has an empty OTLP endpoint"
                )));
            }
        }

        Ok(())
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            receivers: ReceiversConfig::default(),
            processors: BTreeMap::new(),
            exporters: default_exporters(),
            pipelines: default_pipelines(),
        }
    }
}

impl Default for ReceiversConfig {
    fn default() -> Self {
        Self {
            self_observation: SelfObservationReceiverConfig::default(),
            device: DeviceReceiverConfig::default(),
            otlp: OtlpReceiverConfig::default(),
        }
    }
}

impl Default for SelfObservationReceiverConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: DEFAULT_CYCLE_INTERVAL_SECS,
        }
    }
}

impl Default for DeviceReceiverConfig {
    fn default() -> Self {
        match env_or_legacy(
            "SKID_MONITOR_DEVICE_LISTEN_ADDR",
            "MONITOR_CAT_DEVICE_LISTEN_ADDR",
        ) {
            Ok(value)
                if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("disabled") =>
            {
                Self {
                    enabled: false,
                    listen_addr: DEFAULT_DEVICE_LISTEN_ADDR.to_string(),
                }
            }
            Ok(value) => Self {
                enabled: true,
                listen_addr: value,
            },
            Err(_) => Self {
                enabled: true,
                listen_addr: DEFAULT_DEVICE_LISTEN_ADDR.to_string(),
            },
        }
    }
}

impl Default for OtlpReceiverConfig {
    fn default() -> Self {
        match std::env::var("SKID_MONITOR_OTLP_GRPC_ADDR") {
            Ok(value)
                if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("disabled") =>
            {
                Self {
                    enabled: false,
                    grpc_addr: DEFAULT_OTLP_GRPC_ADDR.to_string(),
                }
            }
            Ok(value) => Self {
                enabled: true,
                grpc_addr: value,
            },
            Err(_) => Self {
                enabled: false,
                grpc_addr: DEFAULT_OTLP_GRPC_ADDR.to_string(),
            },
        }
    }
}

impl Default for PipelinesConfig {
    fn default() -> Self {
        default_pipelines()
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            receivers: vec![
                "self_observation".to_string(),
                "device".to_string(),
                "otlp".to_string(),
            ],
            processors: Vec::new(),
            exporters: vec!["skid".to_string()],
        }
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Read { path, source } => {
                write!(f, "read config {}: {source}", path.display())
            }
            ConfigError::Parse { path, source } => {
                write!(f, "parse config {}: {source}", path.display())
            }
            ConfigError::Validate(message) => write!(f, "invalid config: {message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn validate_pipeline(
    signal: &str,
    pipeline: &PipelineConfig,
    config: &AgentConfig,
) -> Result<(), ConfigError> {
    if pipeline.exporters.is_empty() {
        return Err(ConfigError::Validate(format!(
            "{signal} pipeline must have at least one exporter"
        )));
    }

    for receiver in &pipeline.receivers {
        if !matches!(receiver.as_str(), "self_observation" | "device" | "otlp") {
            return Err(ConfigError::Validate(format!(
                "{signal} pipeline references unknown receiver {receiver:?}"
            )));
        }
    }

    for processor in &pipeline.processors {
        if !config.processors.contains_key(processor) {
            return Err(ConfigError::Validate(format!(
                "{signal} pipeline references unknown processor {processor:?}"
            )));
        }
    }

    for exporter in &pipeline.exporters {
        if !config.exporters.contains_key(exporter) {
            return Err(ConfigError::Validate(format!(
                "{signal} pipeline references unknown exporter {exporter:?}"
            )));
        }
    }

    Ok(())
}

fn default_exporters() -> BTreeMap<String, ExporterConfig> {
    let mut exporters = BTreeMap::new();
    exporters.insert(
        "skid".to_string(),
        ExporterConfig::SkidClient {
            addr: env_or_legacy("SKID_MONITOR_CLIENT_ADDR", "MONITOR_CAT_CLIENT_ADDR").ok(),
        },
    );
    exporters
}

fn default_pipelines() -> PipelinesConfig {
    PipelinesConfig {
        metrics: PipelineConfig::default(),
        traces: PipelineConfig::default(),
        logs: PipelineConfig::default(),
    }
}

fn default_true() -> bool {
    true
}

fn default_cycle_interval_secs() -> u64 {
    DEFAULT_CYCLE_INTERVAL_SECS
}

fn default_device_listen_addr() -> String {
    env_or_legacy(
        "SKID_MONITOR_DEVICE_LISTEN_ADDR",
        "MONITOR_CAT_DEVICE_LISTEN_ADDR",
    )
    .ok()
    .filter(|value| !value.eq_ignore_ascii_case("off"))
    .filter(|value| !value.eq_ignore_ascii_case("disabled"))
    .unwrap_or_else(|| DEFAULT_DEVICE_LISTEN_ADDR.to_string())
}

fn default_otlp_receiver_enabled() -> bool {
    std::env::var("SKID_MONITOR_OTLP_GRPC_ADDR")
        .map(|value| !value.eq_ignore_ascii_case("off") && !value.eq_ignore_ascii_case("disabled"))
        .unwrap_or(false)
}

fn default_otlp_grpc_addr() -> String {
    std::env::var("SKID_MONITOR_OTLP_GRPC_ADDR")
        .ok()
        .filter(|value| !value.eq_ignore_ascii_case("off"))
        .filter(|value| !value.eq_ignore_ascii_case("disabled"))
        .unwrap_or_else(|| DEFAULT_OTLP_GRPC_ADDR.to_string())
}

fn default_pipeline_receivers() -> Vec<String> {
    vec![
        "self_observation".to_string(),
        "device".to_string(),
        "otlp".to_string(),
    ]
}

fn default_pipeline_exporters() -> Vec<String> {
    vec!["skid".to_string()]
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_parses_and_validates() {
        let mut config: AgentConfig =
            serde_json::from_str(include_str!("../examples/agent-config.json")).unwrap();

        config.apply_runtime_defaults();
        config.validate().unwrap();

        assert!(config.receivers.self_observation.enabled);
        assert!(config.exporters.contains_key("skid"));
        assert_eq!(config.pipelines.metrics.exporters, ["skid", "debug"]);
    }

    #[test]
    fn unknown_exporter_reference_is_rejected() {
        let mut config: AgentConfig = serde_json::from_str(
            r#"
            {
              "exporters": {
                "debug": { "type": "logging" }
              },
              "pipelines": {
                "metrics": { "exporters": ["missing"] },
                "traces": { "exporters": ["debug"] },
                "logs": { "exporters": ["debug"] }
              }
            }
            "#,
        )
        .unwrap();

        config.apply_runtime_defaults();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("unknown exporter"));
    }
}
