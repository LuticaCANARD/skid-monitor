//! Database log file receiver.
//!
//! Each configured file is tailed and converted into OTLP log records before it enters the
//! regular agent logs pipeline. Offsets are intentionally in-memory: the default `start_at=end`
//! prevents a restart from replaying an entire database log, while `beginning` is available for
//! finite files and backfills.

use crate::config::{DatabaseLogSourceConfig, DatabaseLogsReceiverConfig, LogStartPosition};
use crate::pipeline::{PipelineExportError, ReceiverKind, SignalPipeline};
use skid_protocol::otlp::ExportLogsServiceRequest;
use skid_protocol::otlp::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use skid_protocol::otlp::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber};
use skid_protocol::otlp::tonic::resource::v1::Resource;
use skid_protocol::protocol::Signal;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tracing::{info, warn};

const ROTATION_FINGERPRINT_BYTES: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TailState {
    initialized: bool,
    offset: u64,
    fingerprint: Option<Vec<u8>>,
    pending_line: Vec<u8>,
    pending_line_truncated: bool,
    unavailable: bool,
}

#[derive(Debug)]
enum PollAndExportError {
    Source(io::Error),
    Export(PipelineExportError),
}

pub async fn serve(config: DatabaseLogsReceiverConfig, pipeline: SignalPipeline) {
    let source_count = config.sources.len();
    let mut states = (0..source_count)
        .map(|_| TailState::default())
        .collect::<Vec<_>>();
    let mut interval = tokio::time::interval(Duration::from_millis(config.poll_interval_millis));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    info!(source_count, "database log receiver started");
    loop {
        interval.tick().await;
        for (source, state) in config.sources.iter().zip(&mut states) {
            match poll_and_export_source(
                source,
                state,
                config.start_at,
                config.max_line_bytes,
                config.max_read_bytes,
                &pipeline,
            )
            .await
            {
                Ok(count) => {
                    if state.unavailable {
                        info!(
                            database = %source.system,
                            path = %source.path.display(),
                            "database log source is available"
                        );
                        state.unavailable = false;
                    }
                    if count > 0 {
                        info!(
                            database = %source.system,
                            path = %source.path.display(),
                            count,
                            "collected database logs"
                        );
                    }
                }
                Err(PollAndExportError::Source(err)) => {
                    if !state.unavailable {
                        warn!(
                            database = %source.system,
                            path = %source.path.display(),
                            %err,
                            "database log source is unavailable"
                        );
                        state.unavailable = true;
                    }
                }
                Err(PollAndExportError::Export(err)) => {
                    warn!(
                        database = %source.system,
                        path = %source.path.display(),
                        %err,
                        "database log export failed; tail checkpoint restored for retry"
                    );
                }
            }
        }
    }
}

async fn poll_and_export_source(
    source: &DatabaseLogSourceConfig,
    state: &mut TailState,
    start_at: LogStartPosition,
    max_line_bytes: usize,
    max_read_bytes: usize,
    pipeline: &SignalPipeline,
) -> Result<usize, PollAndExportError> {
    let checkpoint = state.clone();
    let lines = poll_source(source, state, start_at, max_line_bytes, max_read_bytes)
        .await
        .map_err(PollAndExportError::Source)?;
    if lines.is_empty() {
        return Ok(0);
    }

    let count = lines.len();
    let request = export_request(source, lines);
    if let Err(error) = pipeline
        .export(ReceiverKind::DatabaseLogs, Signal::Logs(request))
        .await
    {
        *state = checkpoint;
        return Err(PollAndExportError::Export(error));
    }
    Ok(count)
}

async fn poll_source(
    source: &DatabaseLogSourceConfig,
    state: &mut TailState,
    start_at: LogStartPosition,
    max_line_bytes: usize,
    max_read_bytes: usize,
) -> io::Result<Vec<String>> {
    let mut file = File::open(&source.path).await?;
    let len = file.metadata().await?.len();
    let fingerprint = file_fingerprint(&mut file, len).await?;

    if !state.initialized {
        state.offset = match start_at {
            LogStartPosition::Beginning => 0,
            LogStartPosition::End => len,
        };
        state.fingerprint = fingerprint;
        state.initialized = true;
    } else {
        let truncated = len < state.offset;
        let replaced = state
            .fingerprint
            .as_ref()
            .zip(fingerprint.as_ref())
            .is_some_and(|(previous, current)| fingerprint_replaced(previous, current));
        if truncated || replaced {
            state.offset = 0;
            state.pending_line.clear();
            state.pending_line_truncated = false;
        }
        if fingerprint.is_some() {
            state.fingerprint = fingerprint;
        }
    }

    if state.offset >= len {
        return Ok(Vec::new());
    }

    file.seek(io::SeekFrom::Start(state.offset)).await?;
    let available = usize::try_from(len - state.offset).unwrap_or(usize::MAX);
    let read_len = available.min(max_read_bytes);
    let mut bytes = vec![0; read_len];
    let count = file.read(&mut bytes).await?;
    bytes.truncate(count);
    state.offset = state.offset.saturating_add(count as u64);

    Ok(complete_lines(state, &bytes, max_line_bytes))
}

async fn file_fingerprint(file: &mut File, len: u64) -> io::Result<Option<Vec<u8>>> {
    file.seek(io::SeekFrom::Start(0)).await?;
    let prefix_len = usize::try_from(len)
        .unwrap_or(usize::MAX)
        .min(ROTATION_FINGERPRINT_BYTES);
    let mut prefix = vec![0; prefix_len];
    file.read_exact(&mut prefix).await?;
    Ok(Some(prefix))
}

fn fingerprint_replaced(previous: &[u8], current: &[u8]) -> bool {
    if current.len() >= previous.len() {
        !current.starts_with(previous)
    } else {
        !previous.starts_with(current)
    }
}

fn complete_lines(state: &mut TailState, bytes: &[u8], max_line_bytes: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for &byte in bytes {
        if byte == b'\n' {
            if state.pending_line.last() == Some(&b'\r') {
                state.pending_line.pop();
            }
            if !state.pending_line.is_empty() || state.pending_line_truncated {
                let mut line = String::from_utf8_lossy(&state.pending_line).into_owned();
                if state.pending_line_truncated {
                    line.push_str("… [truncated]");
                }
                lines.push(line);
            }
            state.pending_line.clear();
            state.pending_line_truncated = false;
        } else if state.pending_line.len() < max_line_bytes {
            state.pending_line.push(byte);
        } else {
            state.pending_line_truncated = true;
        }
    }
    lines
}

fn export_request(
    source: &DatabaseLogSourceConfig,
    lines: Vec<String>,
) -> ExportLogsServiceRequest {
    let observed_time_unix_nano = unix_time_nanos();
    let attributes = resource_attributes(source);
    let log_records = lines
        .into_iter()
        .map(|line| log_record(source, line, observed_time_unix_nano))
        .collect();

    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes,
                ..Resource::default()
            }),
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "skid-monitor-agent.database-logs".to_string(),
                    ..InstrumentationScope::default()
                }),
                log_records,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
}

fn resource_attributes(source: &DatabaseLogSourceConfig) -> Vec<KeyValue> {
    let database_system = normalized_database_system(source);
    let service_name = source
        .service_name
        .clone()
        .unwrap_or_else(|| format!("{database_system}-database"));
    let mut attributes = vec![
        string_attribute("service.name", service_name),
        string_attribute("db.system.name", database_system),
        string_attribute("skid_monitor.source", "database_log"),
    ];
    if let Some(namespace) = &source.namespace {
        attributes.push(string_attribute("db.namespace", namespace.clone()));
    }
    if let Some(instance) = &source.instance {
        attributes.push(string_attribute("service.instance.id", instance.clone()));
    }
    attributes
}

fn log_record(
    source: &DatabaseLogSourceConfig,
    line: String,
    observed_time_unix_nano: u64,
) -> LogRecord {
    let (severity_number, severity_text) = severity(&line);
    let mut attributes = vec![
        string_attribute("db.system.name", normalized_database_system(source)),
        string_attribute("log.file.path", source.path.display().to_string()),
    ];
    if let Some(namespace) = &source.namespace {
        attributes.push(string_attribute("db.namespace", namespace.clone()));
    }

    LogRecord {
        time_unix_nano: 0,
        observed_time_unix_nano,
        severity_number: severity_number as i32,
        severity_text: severity_text.to_string(),
        body: Some(AnyValue {
            value: Some(any_value::Value::StringValue(line)),
        }),
        attributes,
        event_name: "database.log".to_string(),
        ..LogRecord::default()
    }
}

fn normalized_database_system(source: &DatabaseLogSourceConfig) -> String {
    source.system.trim().to_ascii_lowercase()
}

fn severity(line: &str) -> (SeverityNumber, &'static str) {
    let line = line.to_ascii_uppercase();
    if line.contains("FATAL") || line.contains("PANIC") {
        (SeverityNumber::Fatal, "FATAL")
    } else if line.contains("ERROR") {
        (SeverityNumber::Error, "ERROR")
    } else if line.contains("WARN") {
        (SeverityNumber::Warn, "WARN")
    } else if line.contains("DEBUG") {
        (SeverityNumber::Debug, "DEBUG")
    } else if line.contains("TRACE") {
        (SeverityNumber::Trace, "TRACE")
    } else {
        (SeverityNumber::Info, "INFO")
    }
}

fn string_attribute(key: impl Into<String>, value: impl Into<String>) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.into())),
        }),
        ..KeyValue::default()
    }
}

fn unix_time_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use std::path::PathBuf;
    use tokio::io::AsyncWriteExt;

    fn source() -> DatabaseLogSourceConfig {
        DatabaseLogSourceConfig {
            system: "postgresql".to_string(),
            path: PathBuf::from("/var/log/postgresql/postgresql.log"),
            namespace: Some("orders".to_string()),
            service_name: None,
            instance: Some("primary".to_string()),
        }
    }

    #[test]
    fn partial_and_oversized_lines_are_completed_safely() {
        let mut state = TailState::default();
        assert!(complete_lines(&mut state, b"ERROR par", 12).is_empty());
        let lines = complete_lines(&mut state, b"t one is too long\nINFO ok\n", 12);

        assert_eq!(lines, ["ERROR part o… [truncated]", "INFO ok"]);
        assert!(state.pending_line.is_empty());
    }

    #[test]
    fn request_contains_database_resource_and_log_attributes() {
        let request = export_request(&source(), vec!["ERROR connection failed".to_string()]);
        let resource_logs = &request.resource_logs[0];
        let resource = resource_logs.resource.as_ref().unwrap();
        let record = &resource_logs.scope_logs[0].log_records[0];

        assert!(has_string_attribute(
            &resource.attributes,
            "db.system.name",
            "postgresql"
        ));
        assert!(has_string_attribute(
            &resource.attributes,
            "db.namespace",
            "orders"
        ));
        assert!(has_string_attribute(
            &record.attributes,
            "log.file.path",
            "/var/log/postgresql/postgresql.log"
        ));
        assert_eq!(record.severity_number, SeverityNumber::Error as i32);
        assert_eq!(record.severity_text, "ERROR");
        assert_eq!(record.event_name, "database.log");
    }

    fn has_string_attribute(attributes: &[KeyValue], key: &str, expected: &str) -> bool {
        attributes.iter().any(|attribute| {
            attribute.key == key
                && matches!(
                    attribute.value.as_ref().and_then(|value| value.value.as_ref()),
                    Some(any_value::Value::StringValue(value)) if value == expected
                )
        })
    }

    #[test]
    fn common_database_severities_are_normalized() {
        assert_eq!(severity("PANIC: crash").0, SeverityNumber::Fatal);
        assert_eq!(severity("[Warning] slow query").0, SeverityNumber::Warn);
        assert_eq!(severity("ready for connections").0, SeverityNumber::Info);
    }

    #[test]
    fn append_growth_is_not_mistaken_for_rotation() {
        assert!(!fingerprint_replaced(b"short", b"short plus append"));
        assert!(fingerprint_replaced(b"old file", b"new file"));
    }

    #[tokio::test]
    async fn tail_starts_at_end_then_collects_append_and_truncate() {
        let path = std::env::temp_dir().join(format!(
            "skid-monitor-db-log-{}-{}.log",
            std::process::id(),
            unix_time_nanos()
        ));
        tokio::fs::write(&path, b"old line\n").await.unwrap();
        let mut source = source();
        source.path = path.clone();
        let mut state = TailState::default();

        assert!(
            poll_source(&source, &mut state, LogStartPosition::End, 1024, 4096)
                .await
                .unwrap()
                .is_empty()
        );

        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        file.write_all(b"ERROR appended\n").await.unwrap();
        file.flush().await.unwrap();
        drop(file);
        assert_eq!(
            poll_source(&source, &mut state, LogStartPosition::End, 1024, 4096)
                .await
                .unwrap(),
            ["ERROR appended"]
        );

        tokio::fs::write(&path, b"new file\n").await.unwrap();
        assert_eq!(
            poll_source(&source, &mut state, LogStartPosition::End, 1024, 4096)
                .await
                .unwrap(),
            ["new file"]
        );

        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn downstream_failure_restores_tail_checkpoint() {
        let path = std::env::temp_dir().join(format!(
            "skid-monitor-db-log-rollback-{}-{}.log",
            std::process::id(),
            unix_time_nanos()
        ));
        tokio::fs::write(&path, b"ERROR retry me\n").await.unwrap();
        let mut source = source();
        source.path = path.clone();
        let mut state = TailState::default();
        let config: AgentConfig = serde_json::from_str(
            r#"
            {
              "exporters": {
                "debug": { "type": "logging" }
              },
              "pipelines": {
                "metrics": { "exporters": ["debug"] },
                "traces": { "exporters": ["debug"] },
                "logs": {
                  "receivers": ["database_logs"],
                  "exporters": ["missing-required-exporter"]
                }
              }
            }
            "#,
        )
        .unwrap();
        let pipeline = SignalPipeline::from_config(&config).unwrap();

        let result = poll_and_export_source(
            &source,
            &mut state,
            LogStartPosition::Beginning,
            1024,
            4096,
            &pipeline,
        )
        .await;

        assert!(matches!(result, Err(PollAndExportError::Export(_))));
        assert_eq!(state, TailState::default());
        assert_eq!(
            poll_source(&source, &mut state, LogStartPosition::Beginning, 1024, 4096)
                .await
                .unwrap(),
            ["ERROR retry me"]
        );

        tokio::fs::remove_file(path).await.unwrap();
    }
}
