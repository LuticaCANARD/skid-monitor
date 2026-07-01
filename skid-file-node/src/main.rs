//! skid file access capability node.
//!
//! 첫 진입점은 실제 파일 전송을 열지 않는다. read-only file offer의 후보 root를 훑고,
//! agent 장비 소켓으로 capability metric만 보낸다.

use skid_protocol::frame;
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};
use skid_protocol::protocol::Signal;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_DEVICE_ADDR: &str = "127.0.0.1:9101";
const DEFAULT_INTERVAL_SECS: u64 = 30;

fn main() {
    let config = FileNodeConfig::from_env_and_args(std::env::args().skip(1));
    eprintln!(
        "skid-file-node starting: node={} roots={} server_device_socket={}",
        config.node_name,
        config.roots.len(),
        config.device_addr
    );

    loop {
        let metrics = sample_file_offer_metrics(&config);
        send(
            Signal::Metrics(export_metrics(
                metrics,
                "skid-file-node",
                "skid-monitor-file",
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
struct FileNodeConfig {
    device_addr: String,
    node_name: String,
    roots: Vec<FileRoot>,
    interval: Duration,
    once: bool,
}

impl FileNodeConfig {
    fn from_env_and_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut roots = Vec::new();
        let mut once = false;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--once" => once = true,
                "--root" => {
                    if let Some(value) = args.next().and_then(parse_root) {
                        roots.push(value);
                    }
                }
                _ if arg.starts_with("--root=") => {
                    if let Some(value) = parse_root(arg.trim_start_matches("--root=")) {
                        roots.push(value);
                    }
                }
                _ => {}
            }
        }

        if roots.is_empty() {
            roots.push(FileRoot {
                label: "cwd".to_string(),
                path: PathBuf::from("."),
            });
        }

        Self {
            device_addr: env_or_legacy("SKID_MONITOR_DEVICE_ADDR", "MONITOR_CAT_DEVICE_ADDR")
                .or_else(|_| {
                    env_or_legacy(
                        "SKID_MONITOR_DEVICE_LISTEN_ADDR",
                        "MONITOR_CAT_DEVICE_LISTEN_ADDR",
                    )
                })
                .unwrap_or_else(|_| DEFAULT_DEVICE_ADDR.to_string()),
            node_name: env_or_legacy("SKID_FILE_NODE_NAME", "MONITOR_CAT_FILE_NODE_NAME")
                .unwrap_or_else(|_| "file-node".to_string()),
            roots,
            interval: Duration::from_secs(
                env_or_legacy(
                    "SKID_FILE_NODE_INTERVAL_SECS",
                    "MONITOR_CAT_FILE_NODE_INTERVAL_SECS",
                )
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|seconds| *seconds > 0)
                .unwrap_or(DEFAULT_INTERVAL_SECS),
            ),
            once,
        }
    }
}

#[derive(Debug, Clone)]
struct FileRoot {
    label: String,
    path: PathBuf,
}

fn parse_root(value: impl AsRef<str>) -> Option<FileRoot> {
    let value = value.as_ref();
    let (label, path) = value.split_once('=')?;
    if label.is_empty() || path.is_empty() {
        return None;
    }

    Some(FileRoot {
        label: label.to_string(),
        path: PathBuf::from(path),
    })
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

fn sample_file_offer_metrics(config: &FileNodeConfig) -> Vec<Metric> {
    let mut metrics = vec![Metric {
        name: "file_node.roots.configured".to_string(),
        value: config.roots.len() as f64,
        source: Source::FileNode,
        unit: None,
        kind: MetricKind::Gauge,
        attributes: vec![("node_name".to_string(), config.node_name.clone())],
    }];

    for root in &config.roots {
        let snapshot = RootSnapshot::read(root);
        metrics.extend(root_metrics(config, root, snapshot));
    }

    metrics
}

fn root_metrics(config: &FileNodeConfig, root: &FileRoot, snapshot: RootSnapshot) -> Vec<Metric> {
    let attrs = || {
        vec![
            ("node_name".to_string(), config.node_name.clone()),
            ("root_label".to_string(), root.label.clone()),
            ("root_path".to_string(), root.path.display().to_string()),
        ]
    };

    vec![
        Metric {
            name: "file_node.root.available".to_string(),
            value: if snapshot.available { 1.0 } else { 0.0 },
            source: Source::FileNode,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: attrs(),
        },
        Metric {
            name: "file_node.root.files".to_string(),
            value: snapshot.file_count as f64,
            source: Source::FileNode,
            unit: None,
            kind: MetricKind::Gauge,
            attributes: attrs(),
        },
        Metric {
            name: "file_node.root.bytes".to_string(),
            value: snapshot.total_bytes as f64,
            source: Source::FileNode,
            unit: Some("By".to_string()),
            kind: MetricKind::Gauge,
            attributes: attrs(),
        },
    ]
}

#[derive(Debug, Clone, Copy)]
struct RootSnapshot {
    available: bool,
    file_count: usize,
    total_bytes: u64,
}

impl RootSnapshot {
    fn read(root: &FileRoot) -> Self {
        let mut snapshot = Self {
            available: false,
            file_count: 0,
            total_bytes: 0,
        };

        let entries = match fs::read_dir(&root.path) {
            Ok(entries) => entries,
            Err(_) => return snapshot,
        };

        snapshot.available = true;
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_file() {
                snapshot.file_count += 1;
                snapshot.total_bytes = snapshot.total_bytes.saturating_add(metadata.len());
            }
        }
        snapshot
    }
}

fn send(signal: Signal, addr: &str) {
    let payload = match frame::encode_signal_payload(&signal) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("signal serialization failed: {err}");
            return;
        }
    };

    match send_tcp(addr, &payload) {
        Ok(()) => eprintln!("sent file node signal: {} bytes", payload.len()),
        Err(err) => eprintln!(
            "failed to send file node signal to {addr}: {err}; payload={}",
            String::from_utf8_lossy(&payload)
        ),
    }
}

fn send_tcp(addr: &str, payload: &[u8]) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(addr)?;
    frame::write_signal_payload(&mut stream, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_labeled_root() {
        let root = parse_root("logs=/var/log").unwrap();

        assert_eq!(root.label, "logs");
        assert_eq!(root.path, PathBuf::from("/var/log"));
    }

    #[test]
    fn rejects_unlabeled_root() {
        assert!(parse_root("/var/log").is_none());
        assert!(parse_root("logs=").is_none());
    }
}
