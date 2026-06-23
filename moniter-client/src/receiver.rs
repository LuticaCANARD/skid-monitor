//! 수신 계층.
//!
//! server agent가 보낸 [`interface::protocol::Signal`]을 받아온다.

use interface::protocol::Signal;
use std::io::{self, Read};
use std::net::TcpListener;

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:9000";
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// client가 수신 대기할 주소.
///
/// server의 `MONITOR_CAT_CLIENT_ADDR`와 같은 값을 쓰면 된다.
pub fn listen_addr() -> String {
    std::env::var("MONITOR_CAT_CLIENT_ADDR").unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_string())
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
        read_signal(&mut stream)
    }
}

fn read_signal(reader: &mut impl Read) -> io::Result<Signal> {
    let mut len_buf = [0_u8; 4];
    reader.read_exact(&mut len_buf)?;

    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }

    let mut payload = vec![0_u8; len as usize];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::metrics::{Metric, MetricKind, Source};

    #[test]
    fn reads_length_prefixed_signal() {
        let signal = Signal::Metrics(vec![Metric {
            name: "cpu.usage".to_string(),
            value: 42.0,
            source: Source::System,
            unit: Some("%".to_string()),
            kind: MetricKind::Gauge,
            attributes: vec![("host".to_string(), "local".to_string())],
        }]);
        let payload = serde_json::to_vec(&signal).unwrap();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);

        let decoded = read_signal(&mut frame.as_slice()).unwrap();
        match decoded {
            Signal::Metrics(metrics) => {
                assert_eq!(metrics.len(), 1);
                assert_eq!(metrics[0].name, "cpu.usage");
            }
            other => panic!("unexpected signal: {other:?}"),
        }
    }

    #[test]
    fn rejects_oversized_frame() {
        let frame = (MAX_FRAME_BYTES + 1).to_be_bytes().to_vec();
        let err = read_signal(&mut frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
