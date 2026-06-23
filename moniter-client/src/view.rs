//! 표시 계층.
//!
//! 수신한 신호를 사용자가 볼 수 있도록 렌더링한다.

use interface::protocol::Signal;

/// 수신한 신호를 사용자에게 보여준다.
pub fn render(signal: &Signal) {
    println!("{}", format_signal(signal));
}

fn format_signal(signal: &Signal) -> String {
    match signal {
        Signal::Metrics(metrics) => {
            let mut lines = vec![format!("metrics: {} items", metrics.len())];
            for metric in metrics {
                let unit = metric.unit.as_deref().unwrap_or("");
                let attrs = format_attrs(&metric.attributes);
                lines.push(format!(
                    "- {} = {}{} [{:?}/{:?}]{}",
                    metric.name, metric.value, unit, metric.source, metric.kind, attrs
                ));
            }
            lines.join("\n")
        }
        Signal::Traces(spans) => {
            let mut lines = vec![format!("traces: {} spans", spans.len())];
            for span in spans {
                let attrs = format_attrs(&span.attributes);
                lines.push(format!(
                    "- {} trace={} span={} {:.2}ms{}",
                    span.name, span.trace_id, span.span_id, span.duration_ms, attrs
                ));
            }
            lines.join("\n")
        }
        Signal::Logs(logs) => {
            let mut lines = vec![format!("logs: {} records", logs.len())];
            for log in logs {
                let severity = if log.severity.is_empty() {
                    "UNKNOWN"
                } else {
                    &log.severity
                };
                let attrs = format_attrs(&log.attributes);
                lines.push(format!("- {severity}: {}{}", log.body, attrs));
            }
            lines.join("\n")
        }
        Signal::Alert { message } => format!("alert: {message}"),
    }
}

fn format_attrs(attributes: &[(String, String)]) -> String {
    if attributes.is_empty() {
        return String::new();
    }

    let rendered = attributes
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(" ({rendered})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::metrics::{Metric, MetricKind, Source};
    use interface::telemetry::{LogRecord, TraceSpan};

    #[test]
    fn formats_metrics() {
        let rendered = format_signal(&Signal::Metrics(vec![Metric {
            name: "requests.total".to_string(),
            value: 3.0,
            source: Source::OpenTelemetry,
            unit: None,
            kind: MetricKind::Sum,
            attributes: vec![("route".to_string(), "/health".to_string())],
        }]));

        assert!(rendered.contains("metrics: 1 items"));
        assert!(rendered.contains("requests.total = 3"));
        assert!(rendered.contains("route=/health"));
    }

    #[test]
    fn formats_traces_and_logs() {
        let trace = format_signal(&Signal::Traces(vec![TraceSpan {
            name: "collect".to_string(),
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            duration_ms: 1.25,
            attributes: Vec::new(),
        }]));
        let log = format_signal(&Signal::Logs(vec![LogRecord {
            severity: "INFO".to_string(),
            body: "started".to_string(),
            attributes: Vec::new(),
        }]));

        assert!(trace.contains("collect trace=abc span=def 1.25ms"));
        assert!(log.contains("INFO: started"));
    }
}
