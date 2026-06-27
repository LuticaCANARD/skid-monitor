//! OTLP wire model shared by server and client.
//!
//! These are generated OpenTelemetry protobuf types with serde support enabled,
//! so the monitor-cat TCP frame can carry OTLP export request payloads without
//! flattening spans, logs, or metrics into lossy DTOs.

pub use opentelemetry_proto::tonic;

pub type ExportMetricsServiceRequest = tonic::collector::metrics::v1::ExportMetricsServiceRequest;
pub type ExportTraceServiceRequest = tonic::collector::trace::v1::ExportTraceServiceRequest;
pub type ExportLogsServiceRequest = tonic::collector::logs::v1::ExportLogsServiceRequest;
