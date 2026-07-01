//! Socket channel for observation devices.
//!
//! Devices can connect to this server-side TCP listener and send the same
//! length-prefixed JSON [`Signal`] frames used by skid-monitor client transport.
//! Accepted signals enter the configured agent pipeline.

use crate::pipeline::{ReceiverKind, SignalPipeline};
use skid_protocol::frame;
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
    pipeline.export(ReceiverKind::Device, signal).await;
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
    frame::decode_signal_payload(&payload)
}
