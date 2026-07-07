//! 수신 계층.
//!
//! server agent가 보낸 [`skid_protocol::protocol::Signal`]을 받아온다.

use skid_protocol::{frame, protocol::Signal};
use std::io;
use std::net::{SocketAddr, TcpListener};

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:9000";
const CLIENT_ADDRS_ENV: &str = "SKID_MONITOR_CLIENT_ADDRS";
const CLIENT_ADDR_ENV: &str = "SKID_MONITOR_CLIENT_ADDR";
const LEGACY_CLIENT_ADDR_ENV: &str = "MONITOR_CAT_CLIENT_ADDR";

/// client가 수신 대기할 주소.
///
/// agent의 `SKID_MONITOR_CLIENT_ADDR`와 같은 값을 쓰면 된다.
pub fn listen_addr() -> String {
    listen_addrs()
        .into_iter()
        .next()
        .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_string())
}

/// client가 수신 대기할 주소 목록.
///
/// `SKID_MONITOR_CLIENT_ADDRS`는 comma-separated list를 받는다. 다중 노드
/// agent를 각각 다른 local port-forward나 overlay 주소로 받을 때 사용한다.
/// agent는 계속 단일 `SKID_MONITOR_CLIENT_ADDR`로 보낸다.
pub fn listen_addrs() -> Vec<String> {
    env_non_empty(CLIENT_ADDRS_ENV)
        .map(|value| parse_addr_list(&value))
        .filter(|addrs| !addrs.is_empty())
        .or_else(|| env_or_legacy(CLIENT_ADDR_ENV, LEGACY_CLIENT_ADDR_ENV).map(|addr| vec![addr]))
        .unwrap_or_else(|| vec![DEFAULT_LISTEN_ADDR.to_string()])
}

fn env_or_legacy(primary: &str, legacy: &str) -> Option<String> {
    env_non_empty(primary).or_else(|| env_non_empty(legacy))
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_addr_list(value: &str) -> Vec<String> {
    let mut addrs = Vec::new();
    for addr in value
        .split(',')
        .map(str::trim)
        .filter(|addr| !addr.is_empty())
    {
        if !addrs.iter().any(|existing| existing == addr) {
            addrs.push(addr.to_string());
        }
    }
    addrs
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

    /// 실제 바인드된 주소를 반환한다.
    ///
    /// 테스트나 port `0` 자동 할당처럼 설정 주소와 OS가 선택한 주소가 다를 때 사용한다.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
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
