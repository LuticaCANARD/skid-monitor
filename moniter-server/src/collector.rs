//! 정보 수집 계층.
//!
//! server 자신을 OpenTelemetry로 계측해 만든 metrics/traces/logs를, in-memory exporter에서 읽어
//! [`interface`] 타입으로 변환한다. 읽기 전 해당 provider를 `force_flush`해서 배치 워커가 모인
//! 신호를 exporter로 내보내게 한 뒤 수집하고, exporter를 reset 한다.

use crate::telemetry::TelemetryGuard;
use interface::metrics::{Metric, MetricKind, Source};
use interface::telemetry::{LogRecord, TraceSpan};
use opentelemetry::global;
use opentelemetry::metrics::Counter;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use std::sync::LazyLock;

/// 수집 주기 횟수 카운터 (server 자체 계측 표본의 예시).
static CYCLE_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("monitor-cat-server")
        .u64_counter("monitor_cat.collect.cycles")
        .with_description("collector::collect 호출 횟수")
        .build()
});

/// 한 주기 동안 수집한 지표를 반환한다.
pub fn collect(guard: &TelemetryGuard) -> Vec<Metric> {
    // server 자체 계측: 이 카운터는 다음 주기의 metric 수집에 잡힌다(self-observation).
    CYCLE_COUNTER.add(1, &[]);

    // 배치/주기 reader가 in-memory exporter로 내보내도록 강제한다.
    let _ = guard.meter_provider.force_flush();

    let mut out = Vec::new();
    if let Ok(resource_metrics) = guard.metric_exporter.get_finished_metrics() {
        for rm in &resource_metrics {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    convert_metric(m.name(), m.unit(), m.data(), &mut out);
                }
            }
        }
    }
    let _ = guard.metric_exporter.reset();
    out
}

/// 완료된 trace span을 수집해 [`TraceSpan`]으로 변환한다.
pub fn collect_spans(guard: &TelemetryGuard) -> Vec<TraceSpan> {
    let _ = guard.tracer_provider.force_flush();

    let mut out = Vec::new();
    if let Ok(spans) = guard.span_exporter.get_finished_spans() {
        for s in &spans {
            let duration_ms = s
                .end_time
                .duration_since(s.start_time)
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            out.push(TraceSpan {
                name: s.name.to_string(),
                trace_id: s.span_context.trace_id().to_string(),
                span_id: s.span_context.span_id().to_string(),
                duration_ms,
                attributes: kv_pairs(s.attributes.iter()),
            });
        }
    }
    let _ = guard.span_exporter.reset();
    out
}

/// 발생한 로그 레코드를 수집해 [`LogRecord`]로 변환한다.
pub fn collect_logs(guard: &TelemetryGuard) -> Vec<LogRecord> {
    let _ = guard.logger_provider.force_flush();

    let mut out = Vec::new();
    if let Ok(logs) = guard.log_exporter.get_emitted_logs() {
        for log in &logs {
            let record = &log.record;
            let severity = record
                .severity_text()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let body = record.body().map(any_value_to_string).unwrap_or_default();
            let attributes = record
                .attributes_iter()
                .map(|(k, v)| (k.to_string(), any_value_to_string(v)))
                .collect();
            out.push(LogRecord {
                severity,
                body,
                attributes,
            });
        }
    }
    let _ = guard.log_exporter.reset();
    out
}

/// OTel metric 하나를 데이터포인트 단위로 [`Metric`]에 펼친다.
fn convert_metric(name: &str, unit: &str, data: &AggregatedMetrics, out: &mut Vec<Metric>) {
    let unit = if unit.is_empty() {
        None
    } else {
        Some(unit.to_string())
    };
    match data {
        AggregatedMetrics::F64(d) => push_data(name, &unit, d, |v| v, out),
        AggregatedMetrics::U64(d) => push_data(name, &unit, d, |v| v as f64, out),
        AggregatedMetrics::I64(d) => push_data(name, &unit, d, |v| v as f64, out),
    }
}

/// 집계 종류별로 데이터포인트를 순회해 `f64` 값으로 평탄화한다.
fn push_data<T: Copy>(
    name: &str,
    unit: &Option<String>,
    data: &MetricData<T>,
    to_f64: impl Fn(T) -> f64,
    out: &mut Vec<Metric>,
) {
    match data {
        MetricData::Gauge(g) => {
            for dp in g.data_points() {
                out.push(make_metric(
                    name,
                    unit,
                    MetricKind::Gauge,
                    to_f64(dp.value()),
                    dp.attributes(),
                ));
            }
        }
        MetricData::Sum(s) => {
            for dp in s.data_points() {
                out.push(make_metric(
                    name,
                    unit,
                    MetricKind::Sum,
                    to_f64(dp.value()),
                    dp.attributes(),
                ));
            }
        }
        MetricData::Histogram(h) => {
            // 히스토그램은 sum을 대표값으로 평탄화한다(버킷 정보는 손실).
            for dp in h.data_points() {
                out.push(make_metric(
                    name,
                    unit,
                    MetricKind::Histogram,
                    to_f64(dp.sum()),
                    dp.attributes(),
                ));
            }
        }
        MetricData::ExponentialHistogram(h) => {
            // 지수 히스토그램도 sum 대표값으로 평탄화한다.
            for dp in h.data_points() {
                out.push(make_metric(
                    name,
                    unit,
                    MetricKind::Histogram,
                    to_f64(dp.sum()),
                    dp.attributes(),
                ));
            }
        }
    }
}

fn make_metric<'a>(
    name: &str,
    unit: &Option<String>,
    kind: MetricKind,
    value: f64,
    attrs: impl Iterator<Item = &'a opentelemetry::KeyValue>,
) -> Metric {
    Metric {
        name: name.to_string(),
        value,
        source: Source::OpenTelemetry,
        unit: unit.clone(),
        kind,
        attributes: kv_pairs(attrs),
    }
}

/// `KeyValue` 목록을 평탄한 (키, 값) 문자열 쌍으로 변환한다.
fn kv_pairs<'a>(attrs: impl Iterator<Item = &'a opentelemetry::KeyValue>) -> Vec<(String, String)> {
    attrs
        .map(|kv| (kv.key.to_string(), kv.value.to_string()))
        .collect()
}

/// OTel `AnyValue`를 사람이 읽을 문자열로 변환한다.
fn any_value_to_string(v: &opentelemetry::logs::AnyValue) -> String {
    use opentelemetry::logs::AnyValue;
    match v {
        AnyValue::Int(i) => i.to_string(),
        AnyValue::Double(d) => d.to_string(),
        AnyValue::String(s) => s.to_string(),
        AnyValue::Boolean(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}
