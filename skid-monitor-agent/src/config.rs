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
    pub database_logs: DatabaseLogsReceiverConfig,
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
#[serde(default, deny_unknown_fields)]
pub struct DatabaseLogsReceiverConfig {
    pub enabled: bool,
    #[serde(default = "default_database_log_poll_interval_millis")]
    pub poll_interval_millis: u64,
    pub start_at: LogStartPosition,
    #[serde(default = "default_database_log_max_line_bytes")]
    pub max_line_bytes: usize,
    #[serde(default = "default_database_log_max_read_bytes")]
    pub max_read_bytes: usize,
    pub sources: Vec<DatabaseLogSourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseLogSourceConfig {
    pub system: String,
    pub path: PathBuf,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogStartPosition {
    Beginning,
    #[default]
    End,
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
        #[serde(default)]
        auth: Option<OtlpAuthConfig>,
    },
}

/// OAuth 2.0 client-credentials settings for a cloud OTLP exporter.
///
/// The secret itself is deliberately not deserializable. `client_secret_env`
/// names the environment variable that contains it at runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpAuthConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret_env: String,
    /// Crash-safe file containing the next cloud ingress sequence to allocate.
    ///
    /// This is required for authenticated exporters so a process restart (or
    /// wall-clock rollback) cannot reuse an already-sent sequence number.
    pub sequence_state_path: PathBuf,
    #[serde(default)]
    pub scope: Option<String>,
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
        if self.receivers.database_logs.poll_interval_millis == 0 {
            self.receivers.database_logs.poll_interval_millis =
                default_database_log_poll_interval_millis();
        }
        if self.receivers.database_logs.max_line_bytes == 0 {
            self.receivers.database_logs.max_line_bytes = default_database_log_max_line_bytes();
        }
        if self.receivers.database_logs.max_read_bytes == 0 {
            self.receivers.database_logs.max_read_bytes = default_database_log_max_read_bytes();
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
            if let ExporterConfig::Otlp { endpoint, auth } = exporter {
                if endpoint.trim().is_empty() {
                    return Err(ConfigError::Validate(format!(
                        "exporter {name:?} has an empty OTLP endpoint"
                    )));
                }
                if let Some(auth) = auth {
                    validate_otlp_auth(name, endpoint, auth)?;
                }
            }
        }

        if self.receivers.database_logs.enabled && self.receivers.database_logs.sources.is_empty() {
            return Err(ConfigError::Validate(
                "database_logs receiver is enabled but has no sources".to_string(),
            ));
        }
        if self.receivers.database_logs.max_line_bytes > 16 * 1024 * 1024 {
            return Err(ConfigError::Validate(
                "database_logs max_line_bytes must not exceed 16 MiB".to_string(),
            ));
        }
        if self.receivers.database_logs.max_read_bytes > 64 * 1024 * 1024 {
            return Err(ConfigError::Validate(
                "database_logs max_read_bytes must not exceed 64 MiB".to_string(),
            ));
        }
        for (index, source) in self.receivers.database_logs.sources.iter().enumerate() {
            if source.system.trim().is_empty() {
                return Err(ConfigError::Validate(format!(
                    "database_logs source {index} has an empty system"
                )));
            }
            if source.path.as_os_str().is_empty() {
                return Err(ConfigError::Validate(format!(
                    "database_logs source {index} has an empty path"
                )));
            }
        }

        Ok(())
    }
}

fn validate_otlp_auth(
    exporter_name: &str,
    endpoint: &str,
    auth: &OtlpAuthConfig,
) -> Result<(), ConfigError> {
    if auth.client_id.trim().is_empty() {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} has an empty OAuth client_id"
        )));
    }
    if auth.client_secret_env.trim().is_empty()
        || auth.client_secret_env.contains('=')
        || auth.client_secret_env.contains('\0')
    {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} has an invalid OAuth client_secret_env"
        )));
    }
    if auth.sequence_state_path.as_os_str().is_empty()
        || auth.sequence_state_path.file_name().is_none()
    {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} has an invalid sequence_state_path"
        )));
    }
    if auth
        .scope
        .as_deref()
        .is_some_and(|scope| scope.trim().is_empty())
    {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} has an empty OAuth scope"
        )));
    }

    let token_url = reqwest::Url::parse(auth.token_url.trim()).map_err(|_| {
        ConfigError::Validate(format!(
            "exporter {exporter_name:?} has an invalid OAuth token_url"
        ))
    })?;
    if token_url.scheme() != "https"
        || token_url.host_str().is_none()
        || !token_url.username().is_empty()
        || token_url.password().is_some()
        || token_url.fragment().is_some()
    {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} OAuth token_url must be an HTTPS URL without credentials or a fragment"
        )));
    }

    // An access token must never be placed on a plaintext OTLP connection.
    let endpoint_url = reqwest::Url::parse(endpoint.trim()).map_err(|_| {
        ConfigError::Validate(format!(
            "exporter {exporter_name:?} authenticated OTLP endpoint must be a valid HTTPS URL"
        ))
    })?;
    if endpoint_url.scheme() != "https"
        || endpoint_url.host_str().is_none()
        || !endpoint_url.username().is_empty()
        || endpoint_url.password().is_some()
        || endpoint_url.fragment().is_some()
    {
        return Err(ConfigError::Validate(format!(
            "exporter {exporter_name:?} authenticated OTLP endpoint must use HTTPS without credentials or a fragment"
        )));
    }

    Ok(())
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
            database_logs: DatabaseLogsReceiverConfig::default(),
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

impl Default for DatabaseLogsReceiverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_millis: default_database_log_poll_interval_millis(),
            start_at: LogStartPosition::End,
            max_line_bytes: default_database_log_max_line_bytes(),
            max_read_bytes: default_database_log_max_read_bytes(),
            sources: Vec::new(),
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
                "database_logs".to_string(),
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
        if !matches!(
            receiver.as_str(),
            "self_observation" | "device" | "otlp" | "database_logs"
        ) {
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
        "database_logs".to_string(),
    ]
}

fn default_database_log_poll_interval_millis() -> u64 {
    1_000
}

fn default_database_log_max_line_bytes() -> usize {
    64 * 1024
}

fn default_database_log_max_read_bytes() -> usize {
    1024 * 1024
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
        assert_eq!(config.receivers.database_logs.sources.len(), 1);
        assert!(config.exporters.contains_key("skid"));
        assert_eq!(config.pipelines.metrics.exporters, ["skid", "debug"]);
        assert!(
            config
                .pipelines
                .logs
                .receivers
                .contains(&"database_logs".to_string())
        );
    }

    #[test]
    fn cloud_sample_config_parses_without_loading_a_secret() {
        let mut config: AgentConfig =
            serde_json::from_str(include_str!("../examples/agent-cloud-config.json")).unwrap();

        config.apply_runtime_defaults();
        config.validate().unwrap();
        assert!(matches!(
            config.exporters.get("cloud"),
            Some(ExporterConfig::Otlp {
                auth: Some(OtlpAuthConfig { client_id, .. }),
                ..
            }) if client_id == "agent-production-01"
        ));
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

    #[test]
    fn legacy_unauthenticated_otlp_exporter_remains_valid() {
        let mut config: AgentConfig = serde_json::from_str(
            r#"
            {
              "exporters": {
                "upstream": { "type": "otlp", "endpoint": "127.0.0.1:4317" }
              },
              "pipelines": {
                "metrics": { "exporters": ["upstream"] },
                "traces": { "exporters": ["upstream"] },
                "logs": { "exporters": ["upstream"] }
              }
            }
            "#,
        )
        .unwrap();

        config.apply_runtime_defaults();
        config.validate().unwrap();
        assert!(matches!(
            config.exporters.get("upstream"),
            Some(ExporterConfig::Otlp { auth: None, .. })
        ));
    }

    #[test]
    fn cloud_otlp_requires_https_for_token_and_signal_endpoints() {
        let parse = |endpoint: &str, token_url: &str| {
            let json = format!(
                r#"{{
                  "exporters": {{
                    "cloud": {{
                      "type": "otlp",
                      "endpoint": "{endpoint}",
                      "auth": {{
                        "token_url": "{token_url}",
                        "client_id": "agent-one",
                        "client_secret_env": "SKID_AGENT_SECRET",
                        "sequence_state_path": "/var/lib/skid-monitor-agent/test.sequence"
                      }}
                    }}
                  }},
                  "pipelines": {{
                    "metrics": {{ "exporters": ["cloud"] }},
                    "traces": {{ "exporters": ["cloud"] }},
                    "logs": {{ "exporters": ["cloud"] }}
                  }}
                }}"#
            );
            let mut config: AgentConfig = serde_json::from_str(&json).unwrap();
            config.apply_runtime_defaults();
            config.validate()
        };

        assert!(
            parse(
                "https://ingress.example.test:4317",
                "http://id.example.test/realms/skid/protocol/openid-connect/token"
            )
            .unwrap_err()
            .to_string()
            .contains("token_url must be an HTTPS URL")
        );
        assert!(
            parse(
                "http://ingress.example.test:4317",
                "https://id.example.test/realms/skid/protocol/openid-connect/token"
            )
            .unwrap_err()
            .to_string()
            .contains("endpoint must use HTTPS")
        );
        assert!(
            parse(
                "https://user:password@ingress.example.test:4317",
                "https://id.example.test/realms/skid/protocol/openid-connect/token"
            )
            .unwrap_err()
            .to_string()
            .contains("without credentials")
        );
        parse(
            "https://ingress.example.test:4317",
            "https://id.example.test/realms/skid/protocol/openid-connect/token",
        )
        .unwrap();
    }

    #[test]
    fn plaintext_client_secret_is_not_a_supported_config_field() {
        let err = serde_json::from_str::<AgentConfig>(
            r#"
            {
              "exporters": {
                "cloud": {
                  "type": "otlp",
                  "endpoint": "https://ingress.example.test:4317",
                  "auth": {
                    "token_url": "https://id.example.test/token",
                    "client_id": "agent-one",
                    "client_secret_env": "SKID_AGENT_SECRET",
                    "sequence_state_path": "/var/lib/skid-monitor-agent/test.sequence",
                    "client_secret": "must-not-be-accepted"
                  }
                }
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field `client_secret`"));
    }

    #[test]
    fn authenticated_otlp_requires_a_sequence_state_path() {
        let err = serde_json::from_str::<AgentConfig>(
            r#"
            {
              "exporters": {
                "cloud": {
                  "type": "otlp",
                  "endpoint": "https://ingress.example.test:4317",
                  "auth": {
                    "token_url": "https://id.example.test/token",
                    "client_id": "agent-one",
                    "client_secret_env": "SKID_AGENT_SECRET"
                  }
                }
              }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("sequence_state_path"));
    }
}
