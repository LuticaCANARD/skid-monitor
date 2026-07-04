//! 수신 계층.
//!
//! server agent가 보낸 [`skid_protocol::protocol::Signal`]을 받아온다.

use skid_protocol::{frame, protocol::Signal};
use std::io;
use std::net::TcpListener;

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:9000";

/// client가 수신 대기할 주소.
///
/// agent의 `SKID_MONITOR_CLIENT_ADDR`와 같은 값을 쓰면 된다.
pub fn listen_addr() -> String {
    env_or_legacy("SKID_MONITOR_CLIENT_ADDR", "MONITOR_CAT_CLIENT_ADDR")
        .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_string())
}

fn env_or_legacy(primary: &str, legacy: &str) -> Option<String> {
    std::env::var(primary)
        .ok()
        .or_else(|| std::env::var(legacy).ok())
}

/// TCP 기반 signal 수신기.
pub struct Receiver {
    listener: TcpListener,
}

impl Receiver {
    /// 지정 주소에 바인드한다.
    pub fn bind(addr: &str) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
        })
    }

    /// 다음 signal 하나를 수신한다.
    ///
    /// server는 signal마다 새 TCP 연결을 열고, `u32` 빅엔디언 길이 프리픽스 뒤에 JSON 본문을 보낸다.
    pub fn recv(&self) -> io::Result<Signal> {
        let (mut stream, _) = self.listener.accept()?;
        frame::read_signal(&mut stream)
    }
}

#[cfg(test)]
#[path = "test/receiver.rs"]
mod tests;
