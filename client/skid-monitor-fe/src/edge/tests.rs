use super::*;

#[test]
fn restores_persisted_edge_state_as_node_summary() {
    let persisted = PersistedEdgeState {
        key: edge_key("127.0.0.1:9000", "edge-a"),
        endpoint: "127.0.0.1:9000".to_string(),
        node: "edge-a".to_string(),
        source: "edge_device".to_string(),
        service: "skid-edge-agent".to_string(),
        metric_points: 7,
        spans: 0,
        log_records: 1,
        last_signal: "metrics".to_string(),
        last_metric: "edge.temperature".to_string(),
        last_value: "31.5".to_string(),
        last_seen_unix_ms: unix_millis(),
        severity: Some(AlertSeverity::Warning),
    };
    let mut decorations = EdgeSignalDecorations::default();

    let nodes = decorations.restore(vec![persisted]);

    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node, "edge-a");
    assert_eq!(
        decorations
            .get("127.0.0.1:9000", "edge-a")
            .and_then(|edge| edge.severity),
        Some(AlertSeverity::Warning)
    );
}
