use super::*;
use crate::model::{DatabaseSystem, MetricSignalSubtype};
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

#[test]
fn mysql_otlp_metrics_are_classified_as_database_subtype() {
    let request = export_metrics(
        vec![Metric {
            name: "db.client.operation.duration".to_string(),
            value: 0.042,
            source: Source::OpenTelemetry,
            unit: Some("s".to_string()),
            kind: MetricKind::Histogram,
            attributes: vec![
                ("db.system.name".to_string(), "mysql".to_string()),
                ("db.namespace".to_string(), "products".to_string()),
                ("db.operation.name".to_string(), "SELECT".to_string()),
                ("server.address".to_string(), "db.local".to_string()),
                ("server.port".to_string(), "3306".to_string()),
            ],
        }],
        "checkout-api",
        "db-client",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(samples[0].signal_subtype, MetricSignalSubtype::Database);
    assert_eq!(samples[0].database_system, Some(DatabaseSystem::MySql));
    assert_eq!(samples[0].database_namespace, "products");
    assert_eq!(samples[0].database_operation, "SELECT");
    assert_eq!(samples[0].database_target, "db.local:3306");
    assert!(samples[0].is_database());
}

#[test]
fn postgresql_otlp_metrics_are_classified_as_database_subtype() {
    let request = export_metrics(
        vec![Metric {
            name: "db.client.connection.count".to_string(),
            value: 12.0,
            source: Source::OpenTelemetry,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: vec![
                ("db.system.name".to_string(), "postgresql".to_string()),
                ("db.namespace".to_string(), "orders|public".to_string()),
                ("server.address".to_string(), "pg.local".to_string()),
                ("server.port".to_string(), "5432".to_string()),
            ],
        }],
        "orders-api",
        "db-client",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(samples[0].signal_subtype, MetricSignalSubtype::Database);
    assert_eq!(samples[0].database_system, Some(DatabaseSystem::PostgreSql));
    assert_eq!(samples[0].database_namespace, "orders|public");
    assert_eq!(samples[0].database_target, "pg.local:5432");
    assert_eq!(samples[0].database_system_label(), "PostgreSQL");
}

#[test]
fn redis_otlp_metrics_are_classified_as_database_subtype() {
    let request = export_metrics(
        vec![Metric {
            name: "db.client.operation.duration".to_string(),
            value: 0.003,
            source: Source::OpenTelemetry,
            unit: Some("s".to_string()),
            kind: MetricKind::Histogram,
            attributes: vec![
                ("db.system.name".to_string(), "redis".to_string()),
                ("db.namespace".to_string(), "0".to_string()),
                ("db.operation.name".to_string(), "GET".to_string()),
                ("server.address".to_string(), "redis.local".to_string()),
                ("server.port".to_string(), "6379".to_string()),
            ],
        }],
        "cache-api",
        "db-client",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(samples[0].signal_subtype, MetricSignalSubtype::Database);
    assert_eq!(samples[0].database_system, Some(DatabaseSystem::Redis));
    assert_eq!(samples[0].database_namespace, "0");
    assert_eq!(samples[0].database_operation, "GET");
    assert_eq!(samples[0].database_target, "redis.local:6379");
    assert_eq!(samples[0].database_system_label(), "Redis");
}

#[test]
fn valkey_otlp_metrics_are_classified_as_database_subtype() {
    let request = export_metrics(
        vec![Metric {
            name: "db.client.operation.duration".to_string(),
            value: 0.004,
            source: Source::OpenTelemetry,
            unit: Some("s".to_string()),
            kind: MetricKind::Histogram,
            attributes: vec![
                ("db.system.name".to_string(), "valkey".to_string()),
                ("db.namespace".to_string(), "1".to_string()),
                ("db.operation.name".to_string(), "SET".to_string()),
                ("server.address".to_string(), "valkey.local".to_string()),
                ("server.port".to_string(), "6379".to_string()),
            ],
        }],
        "cache-api",
        "db-client",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(samples[0].signal_subtype, MetricSignalSubtype::Database);
    assert_eq!(samples[0].database_system, Some(DatabaseSystem::Valkey));
    assert_eq!(samples[0].database_namespace, "1");
    assert_eq!(samples[0].database_operation, "SET");
    assert_eq!(samples[0].database_target, "valkey.local:6379");
    assert_eq!(samples[0].database_system_label(), "Valkey");
}

#[test]
fn unsupported_db_system_stays_a_regular_otlp_metric() {
    let request = export_metrics(
        vec![Metric {
            name: "db.client.operation.duration".to_string(),
            value: 0.02,
            source: Source::OpenTelemetry,
            unit: Some("s".to_string()),
            kind: MetricKind::Histogram,
            attributes: vec![("db.system.name".to_string(), "mongodb".to_string())],
        }],
        "cache-api",
        "db-client",
    );

    let samples = metric_samples(&request, "127.0.0.1:9000");

    assert_eq!(
        samples[0].signal_subtype,
        MetricSignalSubtype::OpenTelemetry
    );
    assert_eq!(samples[0].database_system, None);
    assert!(!samples[0].is_database());
}
