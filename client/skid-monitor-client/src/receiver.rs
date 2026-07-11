//! 수신 계층.
//!
//! server agent가 보낸 [`skid_protocol::protocol::Signal`]을 받아온다.

use skid_protocol::{frame, protocol::Signal};
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:9000";
const CLIENT_ADDRS_ENV: &str = "SKID_MONITOR_CLIENT_ADDRS";
const CLIENT_ADDR_ENV: &str = "SKID_MONITOR_CLIENT_ADDR";
const LEGACY_CLIENT_ADDR_ENV: &str = "MONITOR_CAT_CLIENT_ADDR";
const DEFAULT_FRAME_READ_TIMEOUT: Duration = Duration::from_secs(5);

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
    frame_read_timeout: Duration,
}

impl Receiver {
    /// 지정 주소에 바인드한다.
    ///
    /// This unrestricted binder is retained for compatibility. A remotely
    /// reachable listener must be protected by an authenticated tunnel or an
    /// equivalent external authentication boundary. Solo callers should use
    /// [`Self::bind_trusted_local`].
    pub fn bind(addr: &str) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            frame_read_timeout: DEFAULT_FRAME_READ_TIMEOUT,
        })
    }

    /// Binds a trusted-local listener for solo mode.
    ///
    /// Only numeric IPv4 or IPv6 loopback socket addresses are accepted. This
    /// deliberately rejects wildcard addresses, non-loopback addresses, and
    /// hostnames (including `localhost`) so solo mode cannot be exposed through
    /// DNS resolution or an accidental all-interface bind.
    pub fn bind_trusted_local(addr: &str) -> io::Result<Self> {
        let addr = trusted_local_socket_addr(addr)?;
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            frame_read_timeout: DEFAULT_FRAME_READ_TIMEOUT,
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
        read_signal_with_deadline(&mut stream, self.frame_read_timeout)
    }

    #[cfg(test)]
    fn bind_trusted_local_with_read_timeout(
        addr: &str,
        frame_read_timeout: Duration,
    ) -> io::Result<Self> {
        if frame_read_timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame read timeout must be greater than zero",
            ));
        }

        let addr = trusted_local_socket_addr(addr)?;
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            frame_read_timeout,
        })
    }
}

fn read_signal_with_deadline(
    stream: &mut TcpStream,
    frame_read_timeout: Duration,
) -> io::Result<Signal> {
    stream.set_read_timeout(Some(frame_read_timeout))?;
    let deadline = Instant::now()
        .checked_add(frame_read_timeout)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "frame read timeout overflow")
        })?;
    let mut reader = DeadlineReader { stream, deadline };
    frame::read_signal(&mut reader)
}

struct DeadlineReader<'a> {
    stream: &'a mut TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(frame_read_timeout_error)?;
        self.stream.set_read_timeout(Some(remaining))?;

        match self.stream.read(buf) {
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Err(frame_read_timeout_error())
            }
            result => result,
        }
    }
}

fn frame_read_timeout_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "timed out while reading signal frame",
    )
}

fn trusted_local_socket_addr(addr: &str) -> io::Result<SocketAddr> {
    let socket_addr = addr.parse::<SocketAddr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "trusted-local listener requires a numeric IPv4 or IPv6 loopback socket address; rejected {addr:?}"
            ),
        )
    })?;

    if !socket_addr.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("trusted-local listener requires a loopback address; rejected {socket_addr}"),
        ));
    }

    Ok(socket_addr)
}

#[cfg(test)]
#[path = "test/receiver.rs"]
mod tests;
