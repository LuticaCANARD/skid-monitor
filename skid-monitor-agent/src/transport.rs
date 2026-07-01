//! 전송 계층.
//!
//! 수집한 정보를 [`skid_protocol::protocol::Signal`]에 담아 client로 보낸다.
//!
//! server와 client는 TCP 경계로 나뉠 수 있으므로, 신호를 in-process로 넘기지 않고
//! **JSON으로 직렬화한 뒤 길이 프리픽스 프레이밍으로 TCP 소켓에 전송**한다.
//! 대상 주소는 환경변수 `SKID_MONITOR_CLIENT_ADDR`(예: `127.0.0.1:9000`)로 지정한다.
//! 주소가 없거나 연결에 실패하면 직렬화된 바이트를 로그로 출력한다(client 미구현 단계의 검증용).

use skid_protocol::{frame, protocol::Signal};
use std::net::TcpStream;
use tracing::info;

/// 신호를 configured skid client로 전송한다.
pub fn send_to_client(
    signal: &Signal,
    addr: Option<&str>,
    log_when_missing: bool,
) -> Result<(), String> {
    let payload = match frame::encode_signal_payload(signal) {
        Ok(bytes) => bytes,
        Err(err) => return Err(format!("signal 직렬화 실패: {err}")),
    };

    match addr {
        Some(addr) => {
            send_tcp(addr, &payload).map_err(|err| format!("client로 TCP 전송 실패: {err}"))?;
        }
        // client 주소가 없으면 직렬화 결과를 출력해 변환·직렬화 경로를 검증한다.
        None if log_when_missing => {
            info!(
                bytes = payload.len(),
                json = %String::from_utf8_lossy(&payload),
                "client 미연결: 직렬화된 signal 출력"
            );
        }
        None => {}
    }

    Ok(())
}

/// 길이 프리픽스(빅엔디언 u32) + JSON 본문으로 프레이밍해 전송한다.
fn send_tcp(addr: &str, payload: &[u8]) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(addr)?;
    frame::write_signal_payload(&mut stream, payload)
}
