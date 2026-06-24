//! monitor-cat edge physical signal agent.
//!
//! Edge node 주변의 MCU/센서/전원/네트워크 상태를 `interface` 프로토콜로 client에 전송한다.
//! 첫 구현은 실제 하드웨어 입력 대신 deterministic mock sample을 내보내며, 센서 읽기 계층은
//! 나중에 ESP32/STM32/Linux gateway 구현으로 교체할 수 있게 작게 유지한다.

use interface::metrics::{Metric, MetricKind, Source};
use interface::protocol::Signal;
use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

const DEFAULT_CLIENT_ADDR: &str = "127.0.0.1:9000";
const DEFAULT_INTERVAL_SECS: u64 = 15;

fn main() {
    let config = EdgeConfig::from_env();
    eprintln!(
        "monitor-cat edge agent starting: device={} node={} client={}",
        config.device_id, config.node_name, config.client_addr
    );

    loop {
        let metrics = sample_edge_metrics(&config);
        send(Signal::Metrics(metrics), &config.client_addr);

        if std::env::args().any(|arg| arg == "--once") {
            break;
        }
        std::thread::sleep(config.interval);
    }
}

#[derive(Debug, Clone)]
struct EdgeConfig {
    client_addr: String,
    device_id: String,
    node_name: String,
    interval: Duration,
}

impl EdgeConfig {
    fn from_env() -> Self {
        Self {
            client_addr: std::env::var("MONITOR_CAT_CLIENT_ADDR")
                .unwrap_or_else(|_| DEFAULT_CLIENT_ADDR.to_string()),
            device_id: std::env::var("MONITOR_CAT_EDGE_DEVICE_ID")
                .unwrap_or_else(|_| "edge-dev-001".to_string()),
            node_name: std::env::var("MONITOR_CAT_EDGE_NODE")
                .unwrap_or_else(|_| "edge-node".to_string()),
            interval: Duration::from_secs(
                std::env::var("MONITOR_CAT_EDGE_INTERVAL_SECS")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .filter(|seconds| *seconds > 0)
                    .unwrap_or(DEFAULT_INTERVAL_SECS),
            ),
        }
    }
}

fn sample_edge_metrics(config: &EdgeConfig) -> Vec<Metric> {
    vec![
        make_metric(
            "edge.temperature",
            38.5,
            Some("C"),
            MetricKind::Gauge,
            "enclosure",
            config,
        ),
        make_metric(
            "edge.voltage.input",
            12.1,
            Some("V"),
            MetricKind::Gauge,
            "power",
            config,
        ),
        make_metric(
            "edge.network.rssi",
            -62.0,
            Some("dBm"),
            MetricKind::Gauge,
            "wifi",
            config,
        ),
        make_metric(
            "edge.boot.count",
            1.0,
            None,
            MetricKind::Sum,
            "runtime",
            config,
        ),
        make_metric(
            "edge.watchdog.resets",
            0.0,
            None,
            MetricKind::Sum,
            "runtime",
            config,
        ),
    ]
}

fn make_metric(
    name: &str,
    value: f64,
    unit: Option<&str>,
    kind: MetricKind,
    sensor: &str,
    config: &EdgeConfig,
) -> Metric {
    Metric {
        name: name.to_string(),
        value,
        source: Source::EdgeDevice,
        unit: unit.map(str::to_string),
        kind,
        attributes: vec![
            ("device_id".to_string(), config.device_id.clone()),
            ("node_name".to_string(), config.node_name.clone()),
            ("sensor".to_string(), sensor.to_string()),
        ],
    }
}

fn send(signal: Signal, addr: &str) {
    let payload = match serde_json::to_vec(&signal) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("signal serialization failed: {err}");
            return;
        }
    };

    match send_tcp(addr, &payload) {
        Ok(()) => eprintln!("sent edge signal: {} bytes", payload.len()),
        Err(err) => eprintln!(
            "failed to send edge signal to {addr}: {err}; payload={}",
            String::from_utf8_lossy(&payload)
        ),
    }
}

fn send_tcp(addr: &str, payload: &[u8]) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(addr)?;
    let len = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EdgeConfig {
        EdgeConfig {
            client_addr: "127.0.0.1:9000".to_string(),
            device_id: "dev-a".to_string(),
            node_name: "node-a".to_string(),
            interval: Duration::from_secs(1),
        }
    }

    #[test]
    fn samples_edge_device_metrics() {
        let metrics = sample_edge_metrics(&test_config());

        assert_eq!(metrics.len(), 5);
        assert!(
            metrics
                .iter()
                .all(|metric| matches!(metric.source, Source::EdgeDevice))
        );
        assert!(
            metrics
                .iter()
                .any(|metric| metric.name == "edge.temperature")
        );
        assert!(
            metrics[0]
                .attributes
                .contains(&("device_id".to_string(), "dev-a".to_string()))
        );
    }
}
