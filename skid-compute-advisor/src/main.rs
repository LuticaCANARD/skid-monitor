//! skid compute route advisor node.
//!
//! 첫 진입점은 원격 작업을 실행하지 않는다. 병렬 처리 capability를 관측 신호로 보내고,
//! 나중에 route advisor/scoring을 붙일 수 있는 최소 표면만 연다.

use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};
use skid_protocol::protocol::Signal;
use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

const DEFAULT_DEVICE_ADDR: &str = "127.0.0.1:9101";
const DEFAULT_INTERVAL_SECS: u64 = 30;

fn main() {
    let config = ComputeAdvisorConfig::from_env_and_args(std::env::args().skip(1));
    eprintln!(
        "skid-compute-advisor starting: node={} server_device_socket={}",
        config.node_name, config.device_addr
    );

    loop {
        let metrics = sample_compute_metrics(&config);
        send(
            Signal::Metrics(export_metrics(
                metrics,
                "skid-compute-advisor",
                "skid-monitor-compute",
            )),
            &config.device_addr,
        );

        if config.once {
            break;
        }
        std::thread::sleep(config.interval);
    }
}

#[derive(Debug, Clone)]
struct ComputeAdvisorConfig {
    device_addr: String,
    node_name: String,
    interval: Duration,
    once: bool,
}

impl ComputeAdvisorConfig {
    fn from_env_and_args(args: impl IntoIterator<Item = String>) -> Self {
        Self {
            device_addr: env_or_legacy("SKID_MONITOR_DEVICE_ADDR", "MONITOR_CAT_DEVICE_ADDR")
                .or_else(|_| {
                    env_or_legacy(
                        "SKID_MONITOR_DEVICE_LISTEN_ADDR",
                        "MONITOR_CAT_DEVICE_LISTEN_ADDR",
                    )
                })
                .unwrap_or_else(|_| DEFAULT_DEVICE_ADDR.to_string()),
            node_name: env_or_legacy(
                "SKID_COMPUTE_ADVISOR_NODE",
                "MONITOR_CAT_COMPUTE_ADVISOR_NODE",
            )
            .unwrap_or_else(|_| hostname_fallback()),
            interval: Duration::from_secs(
                env_or_legacy(
                    "SKID_COMPUTE_ADVISOR_INTERVAL_SECS",
                    "MONITOR_CAT_COMPUTE_ADVISOR_INTERVAL_SECS",
                )
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|seconds| *seconds > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            ),
            once: args.into_iter().any(|arg| arg == "--once"),
        }
    }
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "compute-advisor".to_string())
}

fn sample_compute_metrics(config: &ComputeAdvisorConfig) -> Vec<Metric> {
    let logical_cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    let attrs = vec![
        ("node_name".to_string(), config.node_name.clone()),
        ("executor_enabled".to_string(), "false".to_string()),
    ];

    vec![
        Metric {
            name: "compute_advisor.parallelism.logical_cpus".to_string(),
            value: logical_cpus as f64,
            source: Source::ComputeAdvisor,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: attrs.clone(),
        },
        Metric {
            name: "compute_advisor.gpu.detected".to_string(),
            value: 0.0,
            source: Source::ComputeAdvisor,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: attrs.clone(),
        },
        Metric {
            name: "compute_advisor.route.score.placeholder".to_string(),
            value: logical_cpus as f64,
            source: Source::ComputeAdvisor,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: attrs.clone(),
        },
    ]
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
        Ok(()) => eprintln!("sent compute advisor signal: {} bytes", payload.len()),
        Err(err) => eprintln!(
            "failed to send compute advisor signal to {addr}: {err}; payload={}",
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

    #[test]
    fn samples_compute_capability_metrics() {
        let config = ComputeAdvisorConfig {
            device_addr: "127.0.0.1:9101".to_string(),
            node_name: "node-a".to_string(),
            interval: Duration::from_secs(1),
            once: true,
        };

        let metrics = sample_compute_metrics(&config);

        assert_eq!(metrics.len(), 3);
        assert!(
            metrics
                .iter()
                .all(|metric| matches!(metric.source, Source::ComputeAdvisor))
        );
        assert!(
            metrics
                .iter()
                .any(|metric| metric.name == "compute_advisor.parallelism.logical_cpus")
        );
    }
}
