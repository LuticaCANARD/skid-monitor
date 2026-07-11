//! OTLP gRPC receiver.

use crate::pipeline::{PipelineExportError, ReceiverKind, SignalPipeline};
use skid_protocol::otlp::tonic::collector::logs::v1::ExportLogsServiceResponse;
use skid_protocol::otlp::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use skid_protocol::otlp::tonic::collector::metrics::v1::ExportMetricsServiceResponse;
use skid_protocol::otlp::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use skid_protocol::otlp::tonic::collector::trace::v1::ExportTraceServiceResponse;
use skid_protocol::otlp::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use skid_protocol::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use skid_protocol::protocol::Signal;
use std::error::Error;
use std::net::SocketAddr;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

#[derive(Clone)]
struct OtlpIngest {
    pipeline: SignalPipeline,
}

pub async fn serve(
    addr: String,
    pipeline: SignalPipeline,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let addr: SocketAddr = addr.parse()?;
    let ingest = OtlpIngest { pipeline };
    info!(%addr, "OTLP gRPC receiver listening");

    Server::builder()
        .add_service(MetricsServiceServer::new(ingest.clone()))
        .add_service(TraceServiceServer::new(ingest.clone()))
        .add_service(LogsServiceServer::new(ingest))
        .serve(addr)
        .await?;

    Ok(())
}

#[tonic::async_trait]
impl MetricsService for OtlpIngest {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let request = request.into_inner();
        info!(count = metric_count(&request), "received OTLP metrics");
        self.pipeline
            .export(ReceiverKind::Otlp, Signal::Metrics(request))
            .await
            .map_err(downstream_unavailable)?;
        Ok(Response::new(ExportMetricsServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl TraceService for OtlpIngest {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let request = request.into_inner();
        info!(count = span_count(&request), "received OTLP traces");
        self.pipeline
            .export(ReceiverKind::Otlp, Signal::Traces(request))
            .await
            .map_err(downstream_unavailable)?;
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl LogsService for OtlpIngest {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let request = request.into_inner();
        info!(count = log_count(&request), "received OTLP logs");
        self.pipeline
            .export(ReceiverKind::Otlp, Signal::Logs(request))
            .await
            .map_err(downstream_unavailable)?;
        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: None,
        }))
    }
}

fn downstream_unavailable(error: PipelineExportError) -> Status {
    warn!(
        receiver = error.receiver().as_str(),
        signal = error.signal_kind(),
        failure_count = error.failure_count(),
        %error,
        "OTLP signal could not reach every required downstream exporter"
    );
    // Do not expose exporter addresses, OAuth details, or downstream responses
    // to the upstream SDK.
    Status::unavailable("required downstream telemetry exporter is unavailable")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;

    #[tokio::test]
    async fn downstream_failure_returns_safe_unavailable_status() {
        let config: AgentConfig = serde_json::from_str(
            r#"
            {
              "exporters": {
                "debug": { "type": "logging" }
              },
              "pipelines": {
                "metrics": {
                  "receivers": ["otlp"],
                  "exporters": ["secret-downstream-name"]
                },
                "traces": { "exporters": ["debug"] },
                "logs": { "exporters": ["debug"] }
              }
            }
            "#,
        )
        .unwrap();
        let ingest = OtlpIngest {
            pipeline: SignalPipeline::from_config(&config).unwrap(),
        };

        let error = MetricsService::export(
            &ingest,
            Request::new(ExportMetricsServiceRequest::default()),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code(), tonic::Code::Unavailable);
        assert_eq!(
            error.message(),
            "required downstream telemetry exporter is unavailable"
        );
        assert!(!error.message().contains("secret-downstream-name"));
    }
}
