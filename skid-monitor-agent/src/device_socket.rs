//! Socket channel for observation devices.
//!
//! Devices can connect to this server-side TCP listener and send the same
//! length-prefixed JSON [`Signal`] frames used by skid-monitor client transport.
//! The server forwards accepted signals to the configured skid-monitor client.

use crate::transport;
use skid_protocol::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use skid_protocol::protocol::Signal;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

const DEFAULT_DEVICE_LISTEN_ADDR: &str = "127.0.0.1:9101";
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

pub fn listen_addr() -> Option<String> {
    match env_or_legacy(
        "SKID_MONITOR_DEVICE_LISTEN_ADDR",
        "MONITOR_CAT_DEVICE_LISTEN_ADDR",
    ) {
        Ok(value)
            if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("disabled") =>
        {
            None
        }
        Ok(value) => Some(value),
        Err(_) => Some(DEFAULT_DEVICE_LISTEN_ADDR.to_string()),
    }
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

pub async fn serve(addr: String) -> std::io::Result<()> {
    let listener = TcpListener::bind(&addr).await?;
    info!(%addr, "observation device socket listening");

    loop {
        let (stream, peer) = listener.accept().await?;
        info!(%peer, "observation device connected");
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream).await {
                warn!(%peer, %err, "observation device signal rejected");
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    let signal = read_signal(&mut stream).await?;
    match &signal {
        Signal::Metrics(metrics) => info!(count = metric_count(metrics), "received device metrics"),
        Signal::Traces(spans) => info!(count = span_count(spans), "received device traces"),
        Signal::Logs(logs) => info!(count = log_count(logs), "received device logs"),
    }
    transport::send(signal);
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

    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }

    let mut payload = vec![0_u8; len as usize];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}
