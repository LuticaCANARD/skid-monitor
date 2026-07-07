use crate::config;
use crate::model::MetricSample;
use crate::utils::{format_f64, format_metric_value};
use skid_protocol::otlp::tonic::common::v1::{AnyValue, KeyValue, any_value};
use skid_protocol::otlp::tonic::metrics::v1::{Metric as OtlpMetric, metric, number_data_point};

const NODE_ID_ATTRIBUTE_KEYS: [&str; 5] = [
    "node_name",
    "k8s.node.name",
    "host.name",
    "service.instance.id",
    "device_id",
];

pub(crate) fn metric_samples(
    request: &skid_protocol::otlp::ExportMetricsServiceRequest,
    listener: &str,
) -> Vec<MetricSample> {
    let mut samples = Vec::new();
    for resource_metrics in &request.resource_metrics {
        let resource_attrs = resource_metrics
            .resource
            .as_ref()
            .map(|resource| resource.attributes.as_slice())
            .unwrap_or(&[]);
        let source = attribute_value(resource_attrs, config::METRIC_RESOURCE_SOURCE_KEY)
            .unwrap_or_else(|| config::METRIC_UNKNOWN_SOURCE.to_string());
        let service = attribute_value(resource_attrs, config::METRIC_SERVICE_NAME_KEY)
            .unwrap_or_else(|| config::METRIC_EMPTY_FIELD.to_string());

        for scope_metrics in &resource_metrics.scope_metrics {
            let scope = scope_metrics
                .scope
                .as_ref()
                .map(|scope| scope.name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(config::METRIC_EMPTY_FIELD);

            for metric in &scope_metrics.metrics {
                samples.extend(metric_to_samples(
                    metric,
                    &source,
                    &service,
                    scope,
                    resource_attrs,
                    listener,
                ));
            }
        }
    }
    samples
}

fn metric_to_samples(
    metric: &OtlpMetric,
    source: &str,
    service: &str,
    scope: &str,
    resource_attrs: &[KeyValue],
    listener: &str,
) -> Vec<MetricSample> {
    let unit = metric.unit.as_str();

    match &metric.data {
        Some(metric::Data::Gauge(gauge)) => gauge
            .data_points
            .iter()
            .filter_map(|point| {
                point.value.as_ref().map(|value| {
                    let numeric = number_f64(value);
                    metric_sample(
                        &metric.name,
                        format_metric_value(numeric, unit),
                        Some(numeric),
                        source,
                        service,
                        scope,
                        "gauge",
                        resource_attrs,
                        &point.attributes,
                        listener,
                    )
                })
            })
            .collect(),
        Some(metric::Data::Sum(sum)) => sum
            .data_points
            .iter()
            .filter_map(|point| {
                point.value.as_ref().map(|value| {
                    let numeric = number_f64(value);
                    metric_sample(
                        &metric.name,
                        format_metric_value(numeric, unit),
                        Some(numeric),
                        source,
                        service,
                        scope,
                        "sum",
                        resource_attrs,
                        &point.attributes,
                        listener,
                    )
                })
            })
            .collect(),
        Some(metric::Data::Histogram(histogram)) => histogram
            .data_points
            .iter()
            .map(|point| {
                let value = match point.sum {
                    Some(sum) => {
                        format!(
                            "sum {} / count {}",
                            format_metric_value(sum, unit),
                            point.count
                        )
                    }
                    None => format!("count {}", point.count),
                };
                metric_sample(
                    &metric.name,
                    value,
                    point.sum,
                    source,
                    service,
                    scope,
                    "histogram",
                    resource_attrs,
                    &point.attributes,
                    listener,
                )
            })
            .collect(),
        Some(metric::Data::ExponentialHistogram(histogram)) => histogram
            .data_points
            .iter()
            .map(|point| {
                let value = match point.sum {
                    Some(sum) => {
                        format!(
                            "sum {} / count {}",
                            format_metric_value(sum, unit),
                            point.count
                        )
                    }
                    None => format!("count {}", point.count),
                };
                metric_sample(
                    &metric.name,
                    value,
                    point.sum,
                    source,
                    service,
                    scope,
                    "exp_histogram",
                    resource_attrs,
                    &point.attributes,
                    listener,
                )
            })
            .collect(),
        Some(metric::Data::Summary(summary)) => summary
            .data_points
            .iter()
            .map(|point| {
                let value = format!(
                    "sum {} / count {}",
                    format_metric_value(point.sum, unit),
                    point.count
                );
                metric_sample(
                    &metric.name,
                    value,
                    Some(point.sum),
                    source,
                    service,
                    scope,
                    "summary",
                    resource_attrs,
                    &point.attributes,
                    listener,
                )
            })
            .collect(),
        None => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn metric_sample(
    name: &str,
    value: String,
    numeric: Option<f64>,
    source: &str,
    service: &str,
    scope: &str,
    kind: &str,
    resource_attrs: &[KeyValue],
    point_attrs: &[KeyValue],
    listener: &str,
) -> MetricSample {
    let node = node_identity(resource_attrs, point_attrs, service, source, listener);
    MetricSample {
        name: name.to_string(),
        value,
        numeric,
        source: source.to_string(),
        service: service.to_string(),
        node: node.clone(),
        endpoint: listener.to_string(),
        kind: kind.to_string(),
        attributes: metric_attributes(service, scope, point_attrs),
        trend_key: trend_key(listener, name, source, &node, point_attrs),
    }
}

fn metric_attributes(service: &str, scope: &str, attributes: &[KeyValue]) -> String {
    let mut parts = vec![format!("service={service}"), format!("scope={scope}")];
    parts.extend(
        attributes
            .iter()
            .take(config::METRIC_ATTR_PREVIEW_COUNT)
            .map(|attribute| format!("{}={}", attribute.key, key_value(attribute))),
    );
    if attributes.len() > config::METRIC_ATTR_PREVIEW_COUNT {
        parts.push(format!(
            "+{}",
            attributes.len() - config::METRIC_ATTR_PREVIEW_COUNT
        ));
    }
    parts.join(", ")
}

fn trend_key(
    listener: &str,
    name: &str,
    source: &str,
    node: &str,
    attributes: &[KeyValue],
) -> String {
    let mut key = format!("{listener}/{source}/{node}/{name}");
    for attribute in attributes.iter().take(config::METRIC_TREND_ATTR_COUNT) {
        key.push(' ');
        key.push_str(&attribute.key);
        key.push('=');
        key.push_str(&key_value(attribute));
    }
    key
}

fn node_identity(
    resource_attrs: &[KeyValue],
    point_attrs: &[KeyValue],
    service: &str,
    source: &str,
    listener: &str,
) -> String {
    for key in NODE_ID_ATTRIBUTE_KEYS {
        if let Some(value) = attribute_value(point_attrs, key)
            .or_else(|| attribute_value(resource_attrs, key))
            .filter(|value| is_meaningful(value))
        {
            return value;
        }
    }

    if is_meaningful(service) {
        format!("{service}@{listener}")
    } else if is_meaningful(source) {
        format!("{source}@{listener}")
    } else {
        listener.to_string()
    }
}

fn is_meaningful(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value != config::METRIC_EMPTY_FIELD
}

fn attribute_value(attributes: &[KeyValue], key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|attribute| attribute.key == key)
        .map(key_value)
}

fn key_value(attribute: &KeyValue) -> String {
    attribute
        .value
        .as_ref()
        .map(any_value)
        .unwrap_or_else(|| config::METRIC_EMPTY_FIELD.to_string())
}

fn any_value(value: &AnyValue) -> String {
    match &value.value {
        Some(any_value::Value::StringValue(value)) => value.clone(),
        Some(any_value::Value::BoolValue(value)) => value.to_string(),
        Some(any_value::Value::IntValue(value)) => value.to_string(),
        Some(any_value::Value::DoubleValue(value)) => format_f64(*value),
        Some(any_value::Value::ArrayValue(value)) => format!("{} values", value.values.len()),
        Some(any_value::Value::KvlistValue(value)) => format!("{} attrs", value.values.len()),
        Some(any_value::Value::BytesValue(value)) => format!("{} bytes", value.len()),
        Some(any_value::Value::StringValueStrindex(value)) => format!("strindex:{value}"),
        None => config::METRIC_EMPTY_FIELD.to_string(),
    }
}

fn number_f64(value: &number_data_point::Value) -> f64 {
    match value {
        number_data_point::Value::AsDouble(value) => *value,
        number_data_point::Value::AsInt(value) => *value as f64,
    }
}

#[cfg(test)]
mod tests {
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
}
