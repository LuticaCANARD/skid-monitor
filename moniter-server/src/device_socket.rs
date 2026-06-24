//! Socket channel for observation devices.
//!
//! Devices can connect to this server-side TCP listener and send the same
//! length-prefixed JSON [`Signal`] frames used by monitor-cat client transport.
//! The server forwards accepted signals to the configured monitor-cat client.

use crate::transport;
use interface::protocol::Signal;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

const DEFAULT_DEVICE_LISTEN_ADDR: &str = "127.0.0.1:9101";
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

pub fn listen_addr() -> Option<String> {
    match std::env::var("MONITOR_CAT_DEVICE_LISTEN_ADDR") {
        Ok(value)
            if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("disabled") =>
        {
            None
        }
        Ok(value) => Some(value),
        Err(_) => Some(DEFAULT_DEVICE_LISTEN_ADDR.to_string()),
    }
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
        Signal::Metrics(metrics) => info!(count = metrics.len(), "received device metrics"),
        Signal::Traces(spans) => info!(count = spans.len(), "received device traces"),
        Signal::Logs(logs) => info!(count = logs.len(), "received device logs"),
        Signal::Alert { message } => info!(%message, "received device alert"),
    }
    transport::send(signal);
    Ok(())
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
