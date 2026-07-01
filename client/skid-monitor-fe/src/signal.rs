use crate::config;
use crate::model::{MetricSample, ReceiverMessage};
use crate::utils::{format_f64, format_metric_value};
use skid_monitor_client::extension::ExtensionHost;
use skid_monitor_client::receiver::{Receiver as SignalReceiver, listen_addr};
use skid_protocol::otlp::tonic::common::v1::{AnyValue, KeyValue, any_value};
use skid_protocol::otlp::tonic::metrics::v1::{Metric as OtlpMetric, metric, number_data_point};
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub(crate) fn spawn_receiver() -> Receiver<ReceiverMessage> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let addr = listen_addr();
        let mut extension = match ExtensionHost::from_env() {
            Ok(host) => host,
            Err(err) => {
                let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                    "failed to start extension host: {err}"
                )));
                None
            }
        };

        let receiver = match SignalReceiver::bind(&addr) {
            Ok(receiver) => receiver,
            Err(err) => {
                let _ = tx.send(ReceiverMessage::Error(format!(
                    "failed to bind {addr}: {err}"
                )));
                return;
            }
        };

        if tx.send(ReceiverMessage::Listening(addr)).is_err() {
            return;
        }

        loop {
            match receiver.recv() {
                Ok(signal) => {
                    if let Some(extension) = extension.as_mut() {
                        if let Err(err) = extension.publish_signal(&signal) {
                            let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                                "failed to publish to extension host: {err}"
                            )));
                        }
                    }
                    if tx.send(ReceiverMessage::Signal(signal)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    if tx
                        .send(ReceiverMessage::Error(format!("receive error: {err}")))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    rx
}

pub(crate) fn metric_samples(
    request: &skid_protocol::otlp::ExportMetricsServiceRequest,
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
                samples.extend(metric_to_samples(metric, &source, &service, scope));
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
) -> Vec<MetricSample> {
    let unit = metric.unit.as_str();

    match &metric.data {
        Some(metric::Data::Gauge(gauge)) => gauge
            .data_points
            .iter()
            .filter_map(|point| {
                point.value.as_ref().map(|value| MetricSample {
                    name: metric.name.clone(),
                    value: format_metric_value(number_f64(value), unit),
                    numeric: Some(number_f64(value)),
                    source: source.to_string(),
                    kind: "gauge".to_string(),
                    attributes: metric_attributes(service, scope, &point.attributes),
                    trend_key: trend_key(&metric.name, source, &point.attributes),
                })
            })
            .collect(),
        Some(metric::Data::Sum(sum)) => sum
            .data_points
            .iter()
            .filter_map(|point| {
                point.value.as_ref().map(|value| MetricSample {
                    name: metric.name.clone(),
                    value: format_metric_value(number_f64(value), unit),
                    numeric: Some(number_f64(value)),
                    source: source.to_string(),
                    kind: "sum".to_string(),
                    attributes: metric_attributes(service, scope, &point.attributes),
                    trend_key: trend_key(&metric.name, source, &point.attributes),
                })
            })
            .collect(),
        Some(metric::Data::Histogram(histogram)) => histogram
            .data_points
            .iter()
            .map(|point| MetricSample {
                name: metric.name.clone(),
                value: match point.sum {
                    Some(sum) => {
                        format!(
                            "sum {} / count {}",
                            format_metric_value(sum, unit),
                            point.count
                        )
                    }
                    None => format!("count {}", point.count),
                },
                numeric: point.sum,
                source: source.to_string(),
                kind: "histogram".to_string(),
                attributes: metric_attributes(service, scope, &point.attributes),
                trend_key: trend_key(&metric.name, source, &point.attributes),
            })
            .collect(),
        Some(metric::Data::ExponentialHistogram(histogram)) => histogram
            .data_points
            .iter()
            .map(|point| MetricSample {
                name: metric.name.clone(),
                value: match point.sum {
                    Some(sum) => {
                        format!(
                            "sum {} / count {}",
                            format_metric_value(sum, unit),
                            point.count
                        )
                    }
                    None => format!("count {}", point.count),
                },
                numeric: point.sum,
                source: source.to_string(),
                kind: "exp_histogram".to_string(),
                attributes: metric_attributes(service, scope, &point.attributes),
                trend_key: trend_key(&metric.name, source, &point.attributes),
            })
            .collect(),
        Some(metric::Data::Summary(summary)) => summary
            .data_points
            .iter()
            .map(|point| MetricSample {
                name: metric.name.clone(),
                value: format!(
                    "sum {} / count {}",
                    format_metric_value(point.sum, unit),
                    point.count
                ),
                numeric: Some(point.sum),
                source: source.to_string(),
                kind: "summary".to_string(),
                attributes: metric_attributes(service, scope, &point.attributes),
                trend_key: trend_key(&metric.name, source, &point.attributes),
            })
            .collect(),
        None => Vec::new(),
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

fn trend_key(name: &str, source: &str, attributes: &[KeyValue]) -> String {
    let mut key = format!("{source}/{name}");
    for attribute in attributes.iter().take(config::METRIC_TREND_ATTR_COUNT) {
        key.push(' ');
        key.push_str(&attribute.key);
        key.push('=');
        key.push_str(&key_value(attribute));
    }
    key
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
