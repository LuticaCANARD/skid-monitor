//! Legacy length-prefixed JSON `Signal` framing.
//!
//! The current wire contract is a big-endian `u32` payload length followed by a
//! JSON encoded [`Signal`]. SKDM v1 can be added alongside this module later
//! without changing the `Signal` payload contract.

use crate::protocol::Signal;
use std::io::{self, Read, Write};

pub const LEGACY_MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

pub fn encode_signal_payload(signal: &Signal) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(signal)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    validate_payload_len(payload.len())?;
    Ok(payload)
}

pub fn decode_signal_payload(payload: &[u8]) -> io::Result<Signal> {
    serde_json::from_slice(payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn read_signal(reader: &mut impl Read) -> io::Result<Signal> {
    let mut len_buf = [0_u8; 4];
    reader.read_exact(&mut len_buf)?;

    let len = validate_frame_len(u32::from_be_bytes(len_buf))?;
    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    decode_signal_payload(&payload)
}

pub fn write_signal(writer: &mut impl Write, signal: &Signal) -> io::Result<usize> {
    let payload = encode_signal_payload(signal)?;
    write_signal_payload(writer, &payload)?;
    Ok(payload.len())
}

pub fn write_signal_payload(writer: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    validate_payload_len(payload.len())?;
    writer.write_all(&(payload.len() as u32).to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

pub fn validate_frame_len(len: u32) -> io::Result<usize> {
    if len > LEGACY_MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }

    Ok(len as usize)
}

fn validate_payload_len(len: usize) -> io::Result<()> {
    let max = LEGACY_MAX_FRAME_BYTES as usize;
    if len > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} bytes"),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{Metric, MetricKind, Source, export_metrics};

    #[test]
    fn round_trips_legacy_length_prefixed_signal() {
        let signal = Signal::Metrics(export_metrics(
            vec![Metric {
                name: "cpu.usage".to_string(),
                value: 42.0,
                source: Source::System,
                unit: Some("%".to_string()),
                kind: MetricKind::Gauge,
                attributes: vec![("host".to_string(), "local".to_string())],
            }],
            "test-service",
            "test-scope",
        ));

        let mut frame = Vec::new();
        write_signal(&mut frame, &signal).unwrap();
        let decoded = read_signal(&mut frame.as_slice()).unwrap();

        match decoded {
            Signal::Metrics(request) => {
                let metric = &request.resource_metrics[0].scope_metrics[0].metrics[0];
                assert_eq!(metric.name, "cpu.usage");
            }
            _ => panic!("unexpected signal"),
        }
    }

    #[test]
    fn rejects_oversized_frame() {
        let frame = (LEGACY_MAX_FRAME_BYTES + 1).to_be_bytes().to_vec();
        let err = match read_signal(&mut frame.as_slice()) {
            Ok(_) => panic!("oversized frame accepted"),
            Err(err) => err,
        };

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
