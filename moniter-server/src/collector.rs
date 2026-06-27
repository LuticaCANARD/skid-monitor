//! 정보 수집 계층.
//!
//! server 자신을 OpenTelemetry로 계측해 만든 metrics/traces/logs를, in-memory exporter에서 읽어
//! [`interface`] 타입으로 변환한다. 읽기 전 해당 provider를 `force_flush`해서 배치 워커가 모인
//! 신호를 exporter로 내보내게 한 뒤 수집하고, exporter를 reset 한다.

use crate::telemetry::TelemetryGuard;
use interface::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use opentelemetry::global;
use opentelemetry::metrics::Counter;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::logs::LogBatch;
use std::sync::LazyLock;

/// 수집 주기 횟수 카운터 (server 자체 계측 표본의 예시).
static CYCLE_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("monitor-cat-server")
        .u64_counter("monitor_cat.collect.cycles")
        .with_description("collector::collect 호출 횟수")
        .build()
});

/// 한 주기 동안 수집한 지표를 OTLP export request로 반환한다.
pub fn collect(guard: &TelemetryGuard) -> ExportMetricsServiceRequest {
    // server 자체 계측: 이 카운터는 다음 주기의 metric 수집에 잡힌다(self-observation).
    CYCLE_COUNTER.add(1, &[]);

    // 배치/주기 reader가 in-memory exporter로 내보내도록 강제한다.
    let _ = guard.meter_provider.force_flush();

    let mut out = ExportMetricsServiceRequest {
        resource_metrics: Vec::new(),
    };
    if let Ok(resource_metrics) = guard.metric_exporter.get_finished_metrics() {
        for resource_metric in &resource_metrics {
            let request = ExportMetricsServiceRequest::from(resource_metric);
            out.resource_metrics.extend(request.resource_metrics);
        }
    }
    let _ = guard.metric_exporter.reset();
    out
}

/// 완료된 trace span을 수집해 OTLP export request로 변환한다.
pub fn collect_spans(guard: &TelemetryGuard) -> ExportTraceServiceRequest {
    let _ = guard.tracer_provider.force_flush();

    let resource_spans = guard
        .span_exporter
        .get_finished_spans()
        .map(|spans| {
            let resource: ResourceAttributesWithSchema = (&guard.resource).into();
            group_spans_by_resource_and_scope(spans, &resource)
        })
        .unwrap_or_default();
    let _ = guard.span_exporter.reset();

    ExportTraceServiceRequest { resource_spans }
}

/// 발생한 로그 레코드를 수집해 OTLP export request로 변환한다.
pub fn collect_logs(guard: &TelemetryGuard) -> ExportLogsServiceRequest {
    let _ = guard.logger_provider.force_flush();

    let resource_logs = guard
        .log_exporter
        .get_emitted_logs()
        .map(|logs| {
            if logs.is_empty() {
                return Vec::new();
            }

            let resource: ResourceAttributesWithSchema = logs[0].resource.as_ref().into();
            let records = logs
                .iter()
                .map(|log| (&log.record, &log.instrumentation))
                .collect::<Vec<_>>();
            group_logs_by_resource_and_scope(LogBatch::new(&records), &resource)
        })
        .unwrap_or_default();
    let _ = guard.log_exporter.reset();

    ExportLogsServiceRequest { resource_logs }
}
