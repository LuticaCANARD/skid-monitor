use super::*;
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};

#[test]
fn metric_samples_use_explicit_node_name() {
    let request = export_metrics(
        vec![Metric {
            name: "edge.temperature".to_string(),
            value: 31.5,
            source: Source::EdgeDevice,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: vec![("node_name".to_string(), "edge-a".to_string())],
        }],
        "edge-agent",
        "test-scope",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(samples[0].node, "edge-a");
    assert_eq!(samples[0].endpoint, "127.0.0.1:9000");
}

#[test]
fn metric_samples_fallback_to_service_and_listener() {
    let request = export_metrics(
        vec![Metric {
            name: "system.cpu.usage".to_string(),
            value: 12.0,
            source: Source::System,
            unit: Some("%".to_string()),
            kind: MetricKind::Gauge,
            attributes: Vec::new(),
        }],
        "skid-monitor-agent",
        "test-scope",
    );

    let samples = metric_samples(&request, "127.0.0.1:9001");

    assert_eq!(samples[0].node, "skid-monitor-agent@127.0.0.1:9001");
}

#[test]
fn metric_trend_keys_include_listener_endpoint() {
    let request = export_metrics(
        vec![Metric {
            name: "system.cpu.usage".to_string(),
            value: 12.0,
            source: Source::System,
            unit: Some("%".to_string()),
            kind: MetricKind::Gauge,
            attributes: Vec::new(),
        }],
        "skid-monitor-agent",
        "test-scope",
    );

    let first = metric_samples(&request, "127.0.0.1:9000");
    let second = metric_samples(&request, "127.0.0.1:9001");

    assert_ne!(first[0].trend_key, second[0].trend_key);
    assert!(first[0].trend_key.starts_with("127.0.0.1:9000/"));
}
