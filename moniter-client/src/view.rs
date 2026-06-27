//! 표시 계층.
//!
//! 수신한 OTLP 신호를 사용자가 볼 수 있도록 렌더링한다.

use interface::otlp::tonic::common::v1::{AnyValue, KeyValue, any_value};
use interface::otlp::tonic::logs::v1::LogRecord;
use interface::otlp::tonic::metrics::v1::{
    ExponentialHistogram, Histogram, Metric, NumberDataPoint, Sum, metric, number_data_point,
};
use interface::otlp::tonic::trace::v1::Span;
use interface::protocol::Signal;

/// 수신한 신호를 사용자에게 보여준다.
pub fn render(signal: &Signal) {
    println!("{}", format_signal(signal));
}

fn format_signal(signal: &Signal) -> String {
    match signal {
        Signal::Metrics(request) => {
            let mut lines = vec![format!("metrics: {} items", metric_count(request))];
            for resource_metrics in &request.resource_metrics {
                for scope_metrics in &resource_metrics.scope_metrics {
                    for metric in &scope_metrics.metrics {
                        lines.extend(format_metric(metric));
                    }
                }
            }
            lines.join("\n")
        }
        Signal::Traces(request) => {
            let mut lines = vec![format!("traces: {} spans", span_count(request))];
            for resource_spans in &request.resource_spans {
                for scope_spans in &resource_spans.scope_spans {
                    for span in &scope_spans.spans {
                        lines.push(format_span(span));
                    }
                }
            }
            lines.join("\n")
        }
        Signal::Logs(request) => {
            let mut lines = vec![format!("logs: {} records", log_count(request))];
            for resource_logs in &request.resource_logs {
                for scope_logs in &resource_logs.scope_logs {
                    for log in &scope_logs.log_records {
                        lines.push(format_log(log));
                    }
                }
            }
            lines.join("\n")
        }
    }
}

fn metric_count(request: &interface::otlp::ExportMetricsServiceRequest) -> usize {
    request
        .resource_metrics
        .iter()
        .flat_map(|rm| &rm.scope_metrics)
        .map(|sm| sm.metrics.len())
        .sum()
}

fn span_count(request: &interface::otlp::ExportTraceServiceRequest) -> usize {
    request
        .resource_spans
        .iter()
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum()
}

fn log_count(request: &interface::otlp::ExportLogsServiceRequest) -> usize {
    request
        .resource_logs
        .iter()
        .flat_map(|rl| &rl.scope_logs)
        .map(|sl| sl.log_records.len())
        .sum()
}

fn format_metric(metric: &Metric) -> Vec<String> {
    match &metric.data {
        Some(metric::Data::Gauge(gauge)) => {
            format_number_points(metric, "gauge", &gauge.data_points)
        }
        Some(metric::Data::Sum(sum)) => format_sum(metric, sum),
        Some(metric::Data::Histogram(histogram)) => format_histogram(metric, histogram),
        Some(metric::Data::ExponentialHistogram(histogram)) => {
            format_exponential_histogram(metric, histogram)
        }
        Some(metric::Data::Summary(summary)) => vec![format!(
            "- {} summary count={}{}",
            metric.name,
            summary.data_points.len(),
            unit(metric)
        )],
        None => vec![format!("- {} <no data>", metric.name)],
    }
}

fn format_sum(metric: &Metric, sum: &Sum) -> Vec<String> {
    let temporality = match sum.aggregation_temporality {
        value
            if value
                == interface::otlp::tonic::metrics::v1::AggregationTemporality::Delta as i32 =>
        {
            "delta"
        }
        value
            if value
                == interface::otlp::tonic::metrics::v1::AggregationTemporality::Cumulative
                    as i32 =>
        {
            "cumulative"
        }
        _ => "unknown",
    };
    format_number_points(metric, temporality, &sum.data_points)
}

fn format_number_points(metric: &Metric, kind: &str, points: &[NumberDataPoint]) -> Vec<String> {
    points
        .iter()
        .map(|point| {
            let attrs = format_attrs(&point.attributes);
            format!(
                "- {} = {}{} [{}]{}",
                metric.name,
                format_number_value(point),
                unit(metric),
                kind,
                attrs
            )
        })
        .collect()
}

fn format_histogram(metric: &Metric, histogram: &Histogram) -> Vec<String> {
    histogram
        .data_points
        .iter()
        .map(|point| {
            let attrs = format_attrs(&point.attributes);
            let sum = point.sum.unwrap_or_default();
            format!(
                "- {} histogram count={} sum={}{}{}",
                metric.name,
                point.count,
                sum,
                unit(metric),
                attrs
            )
        })
        .collect()
}

fn format_exponential_histogram(metric: &Metric, histogram: &ExponentialHistogram) -> Vec<String> {
    histogram
        .data_points
        .iter()
        .map(|point| {
            let attrs = format_attrs(&point.attributes);
            let sum = point.sum.unwrap_or_default();
            format!(
                "- {} exponential_histogram count={} sum={}{}{}",
                metric.name,
                point.count,
                sum,
                unit(metric),
                attrs
            )
        })
        .collect()
}

fn format_number_value(point: &NumberDataPoint) -> String {
    match point.value {
        Some(number_data_point::Value::AsDouble(value)) => value.to_string(),
        Some(number_data_point::Value::AsInt(value)) => value.to_string(),
        None => "missing".to_string(),
    }
}

fn unit(metric: &Metric) -> &str {
    metric.unit.as_str()
}

fn format_span(span: &Span) -> String {
    let duration_ms = span
        .end_time_unix_nano
        .saturating_sub(span.start_time_unix_nano) as f64
        / 1_000_000.0;
    let attrs = format_attrs(&span.attributes);
    format!(
        "- {} trace={} span={} {:.2}ms{}",
        span.name,
        hex(&span.trace_id),
        hex(&span.span_id),
        duration_ms,
        attrs
    )
}

fn format_log(log: &LogRecord) -> String {
    let severity = if log.severity_text.is_empty() {
        "UNKNOWN"
    } else {
        &log.severity_text
    };
    let body = log.body.as_ref().map(format_any_value).unwrap_or_default();
    let attrs = format_attrs(&log.attributes);
    format!("- {severity}: {body}{attrs}")
}

fn format_attrs(attributes: &[KeyValue]) -> String {
    if attributes.is_empty() {
        return String::new();
    }

    let rendered = attributes
        .iter()
        .map(|kv| {
            let value = kv.value.as_ref().map(format_any_value).unwrap_or_default();
            format!("{}={}", kv.key, value)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" ({rendered})")
}

fn format_any_value(value: &AnyValue) -> String {
    match value.value.as_ref() {
        Some(any_value::Value::StringValue(value)) => value.clone(),
        Some(any_value::Value::BoolValue(value)) => value.to_string(),
        Some(any_value::Value::IntValue(value)) => value.to_string(),
        Some(any_value::Value::DoubleValue(value)) => value.to_string(),
        Some(any_value::Value::ArrayValue(value)) => value
            .values
            .iter()
            .map(format_any_value)
            .collect::<Vec<_>>()
            .join(","),
        Some(any_value::Value::KvlistValue(value)) => value
            .values
            .iter()
            .map(|kv| {
                let value = kv.value.as_ref().map(format_any_value).unwrap_or_default();
                format!("{}={}", kv.key, value)
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(any_value::Value::BytesValue(value)) => hex(value),
        None => String::new(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::metrics::{Metric, MetricKind, Source, export_metrics};
    use interface::otlp::tonic::common::v1::{AnyValue, InstrumentationScope, any_value};
    use interface::otlp::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use interface::otlp::tonic::resource::v1::Resource;
    use interface::otlp::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use interface::otlp::{ExportLogsServiceRequest, ExportTraceServiceRequest};

    #[test]
    fn formats_metrics() {
        let rendered = format_signal(&Signal::Metrics(export_metrics(
            vec![Metric {
                name: "requests.total".to_string(),
                value: 3.0,
                source: Source::OpenTelemetry,
                unit: None,
                kind: MetricKind::Sum,
                attributes: vec![("route".to_string(), "/health".to_string())],
            }],
            "test-service",
            "test-scope",
        )));

        assert!(rendered.contains("metrics: 1 items"));
        assert!(rendered.contains("requests.total = 3"));
        assert!(rendered.contains("route=/health"));
    }

    #[test]
    fn formats_traces_and_logs() {
        let trace = format_signal(&Signal::Traces(ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(empty_resource()),
                scope_spans: vec![ScopeSpans {
                    scope: Some(empty_scope()),
                    spans: vec![Span {
                        trace_id: vec![0xab],
                        span_id: vec![0xde],
                        name: "collect".to_string(),
                        start_time_unix_nano: 1_000_000,
                        end_time_unix_nano: 2_250_000,
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }));
        let log = format_signal(&Signal::Logs(ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(empty_resource()),
                scope_logs: vec![ScopeLogs {
                    scope: Some(empty_scope()),
                    log_records: vec![LogRecord {
                        severity_text: "INFO".to_string(),
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue("started".to_string())),
                        }),
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }));

        assert!(trace.contains("collect trace=ab span=de 1.25ms"));
        assert!(log.contains("INFO: started"));
    }

    fn empty_resource() -> Resource {
        Resource {
            attributes: Vec::new(),
            dropped_attributes_count: 0,
            entity_refs: Vec::new(),
        }
    }

    fn empty_scope() -> InstrumentationScope {
        InstrumentationScope {
            name: "test".to_string(),
            version: String::new(),
            attributes: Vec::new(),
            dropped_attributes_count: 0,
        }
    }
}
