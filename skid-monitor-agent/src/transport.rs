//! 전송 계층.
//!
//! 수집한 정보를 [`skid_protocol::protocol::Signal`]에 담아 client로 보낸다.
//!
//! server와 client는 TCP 경계로 나뉠 수 있으므로, 신호를 in-process로 넘기지 않고
//! **JSON으로 직렬화한 뒤 길이 프리픽스 프레이밍으로 TCP 소켓에 전송**한다.
//! 대상 주소는 환경변수 `SKID_MONITOR_CLIENT_ADDR`(예: `127.0.0.1:9000`)로 지정한다.
//! 주소가 없거나 연결에 실패하면 직렬화된 바이트를 로그로 출력한다(client 미구현 단계의 검증용).

use skid_protocol::protocol::Signal;
use std::io::Write;
use std::net::TcpStream;
use tracing::{info, warn};

/// 신호를 client로 전송한다.
pub fn send(signal: Signal) {
    let payload = match serde_json::to_vec(&signal) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(%err, "signal 직렬화 실패");
            return;
        }
    };

    match env_or_legacy("SKID_MONITOR_CLIENT_ADDR", "MONITOR_CAT_CLIENT_ADDR") {
        Ok(addr) => {
            if let Err(err) = send_tcp(&addr, &payload) {
                warn!(%addr, %err, "client로 TCP 전송 실패");
            }
        }
        // client 주소가 없으면 직렬화 결과를 출력해 변환·직렬화 경로를 검증한다.
        Err(_) => {
            info!(
                bytes = payload.len(),
                json = %String::from_utf8_lossy(&payload),
                "client 미연결: 직렬화된 signal 출력"
            );
        }
    }
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

/// 길이 프리픽스(빅엔디언 u32) + JSON 본문으로 프레이밍해 전송한다.
fn send_tcp(addr: &str, payload: &[u8]) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(addr)?;
    let len = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}
