use super::*;

#[test]
fn cpu_alert_fires_once_until_resolved() {
    let mut alerts = AlertStore::default();
    let hot = sample("system.cpu.usage", 95.0, "95");

    let first = alerts.observe_metric(&hot).expect("first alert");
    assert_eq!(first.transition, AlertTransition::Fired);
    assert_eq!(alerts.summary().active_count, 1);

    assert!(alerts.observe_metric(&hot).is_none());
    assert_eq!(alerts.summary().active_count, 1);

    let cool = sample("system.cpu.usage", 35.0, "35");
    let resolved = alerts.observe_metric(&cool).expect("resolved alert");
    assert_eq!(resolved.transition, AlertTransition::Resolved);
    assert_eq!(alerts.summary().active_count, 0);
}

#[test]
fn file_root_unavailable_is_critical() {
    let mut alerts = AlertStore::default();
    let unavailable = sample("file_node.root.available", 0.0, "0");

    let change = alerts.observe_metric(&unavailable).expect("critical alert");

    assert_eq!(change.snapshot.severity, AlertSeverity::Critical);
    assert_eq!(
        alerts.summary().highest_severity,
        Some(AlertSeverity::Critical)
    );
}

#[test]
fn unknown_metric_does_not_alert() {
    let mut alerts = AlertStore::default();
    assert!(
        alerts
            .observe_metric(&sample("custom.metric", 100.0, "100"))
            .is_none()
    );
}

#[test]
fn same_metric_from_different_listeners_alerts_independently() {
    let mut alerts = AlertStore::default();
    let first = sample_at("system.cpu.usage", 95.0, "95", "127.0.0.1:9000");
    let second = sample_at("system.cpu.usage", 96.0, "96", "127.0.0.1:9001");

    assert!(alerts.observe_metric(&first).is_some());
    assert!(alerts.observe_metric(&second).is_some());
    assert_eq!(alerts.summary().active_count, 2);
    assert_eq!(
        alerts.active_count_for_presenter("127.0.0.1:9000", "agent@127.0.0.1:9000"),
        1
    );

    let recovered = sample_at("system.cpu.usage", 12.0, "12", "127.0.0.1:9000");
    let change = alerts.observe_metric(&recovered).expect("first resolved");

    assert_eq!(change.transition, AlertTransition::Resolved);
    assert_eq!(alerts.summary().active_count, 1);
    assert_eq!(
        alerts.active_count_for_presenter("127.0.0.1:9000", "agent@127.0.0.1:9000"),
        0
    );
    assert_eq!(
        alerts.highest_for_node("127.0.0.1:9001", "agent@127.0.0.1:9001"),
        Some(AlertSeverity::Warning)
    );
}

#[test]
fn receiver_error_applies_to_every_model_on_that_listener_only() {
    let mut alerts = AlertStore::default();
    alerts.observe_receiver_error("127.0.0.1:9000", "listener failed");

    assert_eq!(
        alerts.highest_for_presenter("127.0.0.1:9000", "agent-a"),
        Some(AlertSeverity::Critical)
    );
    assert_eq!(
        alerts.active_count_for_presenter("127.0.0.1:9000", "agent-b"),
        1
    );
    assert_eq!(
        alerts.highest_for_presenter("127.0.0.1:9001", "agent-c"),
        None
    );
}

#[test]
fn extension_error_does_not_change_a_server_presenter() {
    let mut alerts = AlertStore::default();
    alerts.observe_extension_error("sidecar failed");

    assert_eq!(
        alerts.highest_for_presenter("127.0.0.1:9000", "agent-a"),
        None
    );
    assert_eq!(
        alerts.active_count_for_presenter("127.0.0.1:9000", "agent-a"),
        0
    );
    assert_eq!(alerts.summary().active_count, 1);
}

fn sample(name: &str, numeric: f64, value: &str) -> MetricSample {
    sample_at(name, numeric, value, "fixture")
}

fn sample_at(name: &str, numeric: f64, value: &str, endpoint: &str) -> MetricSample {
    MetricSample {
        name: name.to_string(),
        value: value.to_string(),
        numeric: Some(numeric),
        signal_subtype: crate::model::MetricSignalSubtype::OpenTelemetry,
        database_system: None,
        database_namespace: crate::config::METRIC_EMPTY_FIELD.to_string(),
        database_operation: crate::config::METRIC_EMPTY_FIELD.to_string(),
        database_target: crate::config::METRIC_EMPTY_FIELD.to_string(),
        source: "agent".to_string(),
        service: "agent".to_string(),
        node: format!("agent@{endpoint}"),
        endpoint: endpoint.to_string(),
        kind: "gauge".to_string(),
        attributes: "service=agent, scope=test".to_string(),
        trend_key: format!("{endpoint}/agent/agent@{endpoint}/{name}"),
    }
}
