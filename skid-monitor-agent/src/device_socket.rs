//! Socket channel for observation devices.
//!
//! Devices can connect to this server-side TCP listener and send either the
//! legacy length-prefixed JSON [`Signal`] frames used by skid-monitor client
//! transport or the compact edge-wire metrics frame. Accepted signals enter the
//! configured agent pipeline.

use crate::pipeline::{ReceiverKind, SignalPipeline};
use skid_edge_wire as edge_wire;
use skid_edge_wire::{
    EdgeMetricSample, EdgeMetricsFrame, MetricId as EdgeMetricId, SensorId as EdgeSensorId,
};
use skid_protocol::frame;
use skid_protocol::metrics::{Metric, MetricKind as ProtocolMetricKind, Source, export_metrics};
use skid_protocol::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use skid_protocol::protocol::Signal;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

pub async fn serve(addr: String, pipeline: SignalPipeline) -> std::io::Result<()> {
    let listener = TcpListener::bind(&addr).await?;
    info!(%addr, "observation device socket listening");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!(%peer, "observation device connected");
        let pipeline = pipeline.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, pipeline).await {
                warn!(%peer, %err, "observation device signal rejected");
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, pipeline: SignalPipeline) -> std::io::Result<()> {
    let signal = read_signal(&mut stream).await?;
    match &signal {
        Signal::Metrics(metrics) => info!(count = metric_count(metrics), "received device metrics"),
        Signal::Traces(spans) => info!(count = span_count(spans), "received device traces"),
        Signal::Logs(logs) => info!(count = log_count(logs), "received device logs"),
    }
    pipeline
        .export(ReceiverKind::Device, signal)
        .await
        .map_err(std::io::Error::other)?;
    Ok(())
}

fn metric_count(request: &ExportMetricsServiceRequest) -> usize {
    request
        .resource_metrics
        .iter()
        .flat_map(|rm| &rm.scope_metrics)
        .map(|sm| sm.metrics.len())
        .sum()
}

fn span_count(request: &ExportTraceServiceRequest) -> usize {
    request
        .resource_spans
        .iter()
        .flat_map(|rs| &rs.scope_spans)
        .map(|ss| ss.spans.len())
        .sum()
}

fn log_count(request: &ExportLogsServiceRequest) -> usize {
    request
        .resource_logs
        .iter()
        .flat_map(|rl| &rl.scope_logs)
        .map(|sl| sl.log_records.len())
        .sum()
}

async fn read_signal(stream: &mut TcpStream) -> std::io::Result<Signal> {
    let mut len_buf = [0_u8; 4];
    stream.read_exact(&mut len_buf).await?;

    let len = frame::validate_frame_len(u32::from_be_bytes(len_buf))?;

    let mut payload = vec![0_u8; len];
    stream.read_exact(&mut payload).await?;
    decode_device_payload(&payload)
}

fn decode_device_payload(payload: &[u8]) -> std::io::Result<Signal> {
    if edge_wire::is_edge_wire_frame(payload) {
        decode_edge_wire_payload(payload)
    } else {
        frame::decode_signal_payload(payload)
    }
}

fn decode_edge_wire_payload(payload: &[u8]) -> std::io::Result<Signal> {
    let frame = edge_wire::decode_metrics_frame(payload)
        .map_err(|err| invalid_data(format!("edge-wire frame: {err:?}")))?;
    let mut metrics = Vec::with_capacity(frame.metric_count() as usize);

    for sample in frame.records() {
        let sample = sample.map_err(|err| invalid_data(format!("edge-wire record: {err:?}")))?;
        metrics.push(edge_wire_metric(&frame, sample));
    }

    Ok(Signal::Metrics(export_metrics(
        metrics,
        "skid-edge-wire",
        "skid-monitor-edge-wire",
    )))
}

fn edge_wire_metric(frame: &EdgeMetricsFrame<'_>, sample: EdgeMetricSample) -> Metric {
    let mut attributes = vec![
        ("device_id".to_string(), frame.device_id().to_string()),
        ("node_name".to_string(), frame.node_name().to_string()),
        ("metric_id".to_string(), sample.metric_id.get().to_string()),
        ("sensor_id".to_string(), sample.sensor_id.get().to_string()),
    ];

    if let Some(sensor) = sensor_name(sample.sensor_id) {
        attributes.push(("sensor".to_string(), sensor.to_string()));
    }

    Metric {
        name: metric_name(sample.metric_id),
        value: sample.value as f64,
        source: Source::EdgeDevice,
        unit: sample.unit.as_str().map(str::to_string),
        kind: match sample.kind {
            edge_wire::MetricKind::Gauge => ProtocolMetricKind::Gauge,
            edge_wire::MetricKind::Sum => ProtocolMetricKind::Sum,
            edge_wire::MetricKind::Histogram => ProtocolMetricKind::Histogram,
        },
        attributes,
    }
}

fn metric_name(metric_id: EdgeMetricId) -> String {
    metric_id
        .known_name()
        .map(str::to_string)
        .unwrap_or_else(|| format!("edge.metric.{}", metric_id.get()))
}

fn sensor_name(sensor_id: EdgeSensorId) -> Option<&'static str> {
    sensor_id.known_name()
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skid_edge_wire::{EdgeIdentity, MetricKind as EdgeMetricKind, SensorId, Unit};
    use skid_protocol::otlp::tonic::metrics::v1::metric;

    #[test]
    fn decodes_compact_edge_wire_payload_into_signal_metrics() {
        let identity = EdgeIdentity {
            device_id: "esp32-a",
            node_name: "rack-a",
        };
        let samples = [EdgeMetricSample {
            metric_id: EdgeMetricId::EDGE_TEMPERATURE,
            sensor_id: SensorId::ENCLOSURE,
            kind: EdgeMetricKind::Gauge,
            unit: Unit::Celsius,
            value: 38.5,
        }];
        let mut payload = [0_u8; 96];
        let len = edge_wire::encode_metrics_frame(&mut payload, 11, identity, &samples).unwrap();

        let signal = decode_device_payload(&payload[..len]).unwrap();
        let Signal::Metrics(request) = signal else {
            panic!("expected metrics signal");
        };

        let resource_metrics = &request.resource_metrics[0];
        let metric = &resource_metrics.scope_metrics[0].metrics[0];
        assert_eq!(metric.name, "edge.temperature");
        assert_eq!(metric.unit, "C");

        let attrs = match metric.data.as_ref().unwrap() {
            metric::Data::Gauge(gauge) => &gauge.data_points[0].attributes,
            _ => panic!("expected gauge metric"),
        };
        assert!(attrs.iter().any(|attr| attr.key == "device_id"));
        assert!(attrs.iter().any(|attr| attr.key == "node_name"));
        assert!(attrs.iter().any(|attr| attr.key == "sensor"));
    }
}
