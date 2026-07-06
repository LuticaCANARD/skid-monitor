//! 수집되는 간단 metric 표본과 OTLP 변환 헬퍼.
//!
//! server-client 경계의 실제 payload는 OTLP `ExportMetricsServiceRequest`다.
//! 이 모듈의 [`Metric`]은 system/edge처럼 SDK aggregator를 거치지 않는 값들을
//! OTLP number data point로 감싸기 위한 작은 입력 타입이다.

use crate::otlp::ExportMetricsServiceRequest;
use crate::otlp::tonic::common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value};
use crate::otlp::tonic::metrics::v1::{
    AggregationTemporality, Gauge, Histogram, HistogramDataPoint, Metric as OtlpMetric,
    NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric, number_data_point,
};
use crate::otlp::tonic::resource::v1::Resource;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 단일 측정 표본.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// 측정 이름 (예: "cpu.usage", "pod.restart_count").
    pub name: String,
    /// 측정 값. SDK aggregator를 거치지 않은 단일 표본 값이다.
    pub value: f64,
    /// 측정이 발생한 출처.
    pub source: Source,
    /// 측정 단위 (예: "ms", "By"). 없을 수 있다.
    pub unit: Option<String>,
    /// 집계 종류.
    pub kind: MetricKind,
    /// OpenTelemetry attribute 등 부가 속성을 평탄화한 키-값 목록.
    pub attributes: Vec<(String, String)>,
}

/// 측정이 발생한 출처 구분.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    /// OpenTelemetry 계측에서 온 값.
    OpenTelemetry,
    /// 쿠버네티스 인프라/시스템에서 온 값.
    Kubernetes,
    /// 엣지 노드 주변의 MCU/센서/전원/네트워크 장비에서 온 물리 계층 신호.
    EdgeDevice,
    /// 파일 offer, 안전한 root, 전송 가능성 등 파일 접근 노드에서 온 신호.
    FileNode,
    /// 병렬 처리 capability와 route advice 후보 산정을 위한 compute 노드 신호.
    ComputeAdvisor,
    /// 클라우드 QPU 작업 상태, 결과 품질, 큐 상태 등 양자 컴퓨팅 백엔드에서 온 신호.
    Quantum,
    /// 그 외 호스트 시스템 지표.
    System,
    /// macOS/MacBook host 지표. Linux `system` source와 분리해 UI에서 별도 live signal로 볼 수 있다.
    MacOS,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::OpenTelemetry => "opentelemetry",
            Source::Kubernetes => "kubernetes",
            Source::EdgeDevice => "edge_device",
            Source::FileNode => "file_node",
            Source::ComputeAdvisor => "compute_advisor",
            Source::Quantum => "quantum",
            Source::System => "system",
            Source::MacOS => "macos",
        }
    }
}

/// 메트릭 집계 종류.
///
/// SDK aggregator를 거치지 않은 간단 표본의 집계 종류.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MetricKind {
    /// 현재 값 측정.
    Gauge,
    /// 누적 합계.
    Sum,
    /// 단일 표본을 담은 히스토그램.
    Histogram,
}

/// SDK aggregator를 거치지 않은 metric 표본을 OTLP Metrics export request로 변환한다.
pub fn export_metrics(
    metrics: Vec<Metric>,
    service_name: &str,
    scope_name: &str,
) -> ExportMetricsServiceRequest {
    if metrics.is_empty() {
        return ExportMetricsServiceRequest {
            resource_metrics: Vec::new(),
        };
    }

    let time_unix_nano = unix_nanos(SystemTime::now());
    let mut groups: Vec<(Source, Vec<Metric>)> = Vec::new();
    for metric in metrics {
        if let Some((_, grouped)) = groups
            .iter_mut()
            .find(|(source, _)| *source == metric.source)
        {
            grouped.push(metric);
        } else {
            groups.push((metric.source, vec![metric]));
        }
    }

    ExportMetricsServiceRequest {
        resource_metrics: groups
            .into_iter()
            .map(|(source, metrics)| {
                resource_metrics(source, metrics, service_name, scope_name, time_unix_nano)
            })
            .collect(),
    }
}

fn resource_metrics(
    source: Source,
    metrics: Vec<Metric>,
    service_name: &str,
    scope_name: &str,
    time_unix_nano: u64,
) -> ResourceMetrics {
    ResourceMetrics {
        resource: Some(Resource {
            attributes: key_values([
                ("service.name".to_string(), service_name.to_string()),
                (
                    "skid_monitor.source".to_string(),
                    source.as_str().to_string(),
                ),
            ]),
            dropped_attributes_count: 0,
            entity_refs: Vec::new(),
        }),
        scope_metrics: vec![ScopeMetrics {
            scope: Some(InstrumentationScope {
                name: scope_name.to_string(),
                version: String::new(),
                attributes: Vec::new(),
                dropped_attributes_count: 0,
            }),
            metrics: metrics
                .into_iter()
                .map(|metric| otlp_metric(metric, time_unix_nano))
                .collect(),
            schema_url: String::new(),
        }],
        schema_url: String::new(),
    }
}

fn otlp_metric(metric: Metric, time_unix_nano: u64) -> OtlpMetric {
    let name = metric.name;
    let unit = metric.unit.unwrap_or_default();
    let kind = metric.kind;
    let value = metric.value;
    let attributes = key_values(metric.attributes);
    OtlpMetric {
        name,
        description: String::new(),
        unit,
        metadata: Vec::new(),
        data: Some(match kind {
            MetricKind::Gauge => metric::Data::Gauge(Gauge {
                data_points: vec![number_data_point(attributes, value, 0, time_unix_nano)],
            }),
            MetricKind::Sum => metric::Data::Sum(Sum {
                data_points: vec![number_data_point(
                    attributes,
                    value,
                    time_unix_nano,
                    time_unix_nano,
                )],
                aggregation_temporality: AggregationTemporality::Cumulative as i32,
                is_monotonic: true,
            }),
            MetricKind::Histogram => metric::Data::Histogram(Histogram {
                data_points: vec![HistogramDataPoint {
                    attributes,
                    start_time_unix_nano: time_unix_nano,
                    time_unix_nano,
                    count: 1,
                    sum: Some(value),
                    bucket_counts: Vec::new(),
                    explicit_bounds: Vec::new(),
                    exemplars: Vec::new(),
                    flags: 0,
                    min: Some(value),
                    max: Some(value),
                }],
                aggregation_temporality: AggregationTemporality::Cumulative as i32,
            }),
        }),
    }
}

fn number_data_point(
    attributes: Vec<KeyValue>,
    value: f64,
    start_time_unix_nano: u64,
    time_unix_nano: u64,
) -> NumberDataPoint {
    NumberDataPoint {
        attributes,
        start_time_unix_nano,
        time_unix_nano,
        exemplars: Vec::new(),
        flags: 0,
        value: Some(number_data_point::Value::AsDouble(value)),
    }
}

fn key_values(attributes: impl IntoIterator<Item = (String, String)>) -> Vec<KeyValue> {
    let mut out = Vec::new();
    for (key, value) in attributes {
        if out.iter().any(|existing: &KeyValue| existing.key == key) {
            continue;
        }
        out.push(KeyValue {
            key,
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(value)),
            }),
            key_strindex: 0,
        });
    }
    out
}

fn unix_nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_nanos() as u64
}
