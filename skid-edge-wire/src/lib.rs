#![no_std]
//! Compact edge-device wire interface.
//!
//! This crate is intentionally independent from `skid-protocol`, OTLP, JSON,
//! sockets, clocks, allocation, and operating-system APIs. MCU and RTOS
//! firmware can use it to encode metric samples into a caller-provided buffer.
//! Host-side agents can decode the same frame and lift it into the canonical
//! `Signal::Metrics` / OTLP model.

use core::str;

pub const MAGIC: [u8; 2] = *b"SE";
pub const VERSION: u8 = 1;
pub const FRAME_KIND_METRICS: u8 = 1;
pub const HEADER_LEN: usize = 10;
pub const RECORD_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    FieldTooLong,
    TooManyMetrics,
    OutputTooSmall,
    ReservedMetricId,
    ReservedSensorId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    BadMagic,
    UnsupportedVersion,
    UnsupportedKind,
    InvalidUtf8,
    TruncatedIdentity,
    BadRecordLength,
    ReservedMetricId,
    ReservedSensorId,
    UnknownMetricKind,
    UnknownUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeIdentity<'a> {
    pub device_id: &'a str,
    pub node_name: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeMetricSample {
    pub metric_id: MetricId,
    pub sensor_id: SensorId,
    pub kind: MetricKind,
    pub unit: Unit,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricId(u16);

impl MetricId {
    pub const EDGE_TEMPERATURE: Self = Self(1);
    pub const EDGE_VOLTAGE_INPUT: Self = Self(2);
    pub const EDGE_NETWORK_RSSI: Self = Self(3);
    pub const EDGE_BOOT_COUNT: Self = Self(4);
    pub const EDGE_WATCHDOG_RESETS: Self = Self(5);

    pub const fn new(value: u16) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn known_name(self) -> Option<&'static str> {
        match self.0 {
            1 => Some("edge.temperature"),
            2 => Some("edge.voltage.input"),
            3 => Some("edge.network.rssi"),
            4 => Some("edge.boot.count"),
            5 => Some("edge.watchdog.resets"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorId(u16);

impl SensorId {
    pub const ENCLOSURE: Self = Self(1);
    pub const POWER: Self = Self(2);
    pub const WIFI: Self = Self(3);
    pub const RUNTIME: Self = Self(4);

    pub const fn new(value: u16) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn known_name(self) -> Option<&'static str> {
        match self.0 {
            1 => Some("enclosure"),
            2 => Some("power"),
            3 => Some("wifi"),
            4 => Some("runtime"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MetricKind {
    Gauge = 1,
    Sum = 2,
    Histogram = 3,
}

impl MetricKind {
    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Gauge),
            2 => Some(Self::Sum),
            3 => Some(Self::Histogram),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Unit {
    None = 0,
    Celsius = 1,
    Volt = 2,
    Dbm = 3,
    Count = 4,
    Percent = 5,
}

impl Unit {
    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Celsius),
            2 => Some(Self::Volt),
            3 => Some(Self::Dbm),
            4 => Some(Self::Count),
            5 => Some(Self::Percent),
            _ => None,
        }
    }

    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Celsius => Some("C"),
            Self::Volt => Some("V"),
            Self::Dbm => Some("dBm"),
            Self::Count => Some("1"),
            Self::Percent => Some("%"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeMetricsFrame<'a> {
    seq: u16,
    flags: u8,
    device_id: &'a str,
    node_name: &'a str,
    metric_count: u8,
    records: &'a [u8],
}

impl<'a> EdgeMetricsFrame<'a> {
    pub const fn seq(&self) -> u16 {
        self.seq
    }

    pub const fn flags(&self) -> u8 {
        self.flags
    }

    pub const fn device_id(&self) -> &'a str {
        self.device_id
    }

    pub const fn node_name(&self) -> &'a str {
        self.node_name
    }

    pub const fn metric_count(&self) -> u8 {
        self.metric_count
    }

    pub const fn records(&self) -> EdgeMetricIter<'a> {
        EdgeMetricIter {
            records: self.records,
            remaining: self.metric_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EdgeMetricIter<'a> {
    records: &'a [u8],
    remaining: u8,
}

impl Iterator for EdgeMetricIter<'_> {
    type Item = Result<EdgeMetricSample, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        if self.records.len() < RECORD_LEN {
            self.remaining = 0;
            return Some(Err(DecodeError::BadRecordLength));
        }

        let record = &self.records[..RECORD_LEN];
        self.records = &self.records[RECORD_LEN..];
        self.remaining -= 1;

        let metric_id = match MetricId::new(u16::from_be_bytes([record[0], record[1]])) {
            Some(value) => value,
            None => return Some(Err(DecodeError::ReservedMetricId)),
        };
        let sensor_id = match SensorId::new(u16::from_be_bytes([record[2], record[3]])) {
            Some(value) => value,
            None => return Some(Err(DecodeError::ReservedSensorId)),
        };
        let kind = match MetricKind::from_u8(record[4]) {
            Some(value) => value,
            None => return Some(Err(DecodeError::UnknownMetricKind)),
        };
        let unit = match Unit::from_u8(record[5]) {
            Some(value) => value,
            None => return Some(Err(DecodeError::UnknownUnit)),
        };
        let value = f32::from_bits(u32::from_be_bytes([
            record[6], record[7], record[8], record[9],
        ]));

        Some(Ok(EdgeMetricSample {
            metric_id,
            sensor_id,
            kind,
            unit,
            value,
        }))
    }
}

pub fn is_edge_wire_frame(input: &[u8]) -> bool {
    input.len() >= HEADER_LEN && input[0..2] == MAGIC
}

pub fn encode_metrics_frame(
    out: &mut [u8],
    seq: u16,
    identity: EdgeIdentity<'_>,
    metrics: &[EdgeMetricSample],
) -> Result<usize, EncodeError> {
    let device_id = identity.device_id.as_bytes();
    let node_name = identity.node_name.as_bytes();

    if device_id.len() > u8::MAX as usize || node_name.len() > u8::MAX as usize {
        return Err(EncodeError::FieldTooLong);
    }
    if metrics.len() > u8::MAX as usize {
        return Err(EncodeError::TooManyMetrics);
    }
    if metrics.iter().any(|sample| sample.metric_id.get() == 0) {
        return Err(EncodeError::ReservedMetricId);
    }
    if metrics.iter().any(|sample| sample.sensor_id.get() == 0) {
        return Err(EncodeError::ReservedSensorId);
    }

    let required = HEADER_LEN + device_id.len() + node_name.len() + (metrics.len() * RECORD_LEN);
    if out.len() < required {
        return Err(EncodeError::OutputTooSmall);
    }

    out[0..2].copy_from_slice(&MAGIC);
    out[2] = VERSION;
    out[3] = FRAME_KIND_METRICS;
    out[4] = 0;
    out[5..7].copy_from_slice(&seq.to_be_bytes());
    out[7] = metrics.len() as u8;
    out[8] = device_id.len() as u8;
    out[9] = node_name.len() as u8;

    let mut cursor = HEADER_LEN;
    out[cursor..cursor + device_id.len()].copy_from_slice(device_id);
    cursor += device_id.len();
    out[cursor..cursor + node_name.len()].copy_from_slice(node_name);
    cursor += node_name.len();

    for sample in metrics {
        out[cursor..cursor + 2].copy_from_slice(&sample.metric_id.get().to_be_bytes());
        out[cursor + 2..cursor + 4].copy_from_slice(&sample.sensor_id.get().to_be_bytes());
        out[cursor + 4] = sample.kind as u8;
        out[cursor + 5] = sample.unit as u8;
        out[cursor + 6..cursor + 10].copy_from_slice(&sample.value.to_bits().to_be_bytes());
        cursor += RECORD_LEN;
    }

    Ok(required)
}

pub fn decode_metrics_frame(input: &[u8]) -> Result<EdgeMetricsFrame<'_>, DecodeError> {
    if input.len() < HEADER_LEN {
        return Err(DecodeError::TooShort);
    }
    if input[0..2] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    if input[2] != VERSION {
        return Err(DecodeError::UnsupportedVersion);
    }
    if input[3] != FRAME_KIND_METRICS {
        return Err(DecodeError::UnsupportedKind);
    }

    let flags = input[4];
    let seq = u16::from_be_bytes([input[5], input[6]]);
    let metric_count = input[7];
    let device_id_len = input[8] as usize;
    let node_name_len = input[9] as usize;
    let identity_len = device_id_len + node_name_len;

    if input.len() < HEADER_LEN + identity_len {
        return Err(DecodeError::TruncatedIdentity);
    }

    let mut cursor = HEADER_LEN;
    let device_id = str::from_utf8(&input[cursor..cursor + device_id_len])
        .map_err(|_| DecodeError::InvalidUtf8)?;
    cursor += device_id_len;
    let node_name = str::from_utf8(&input[cursor..cursor + node_name_len])
        .map_err(|_| DecodeError::InvalidUtf8)?;
    cursor += node_name_len;

    let records = &input[cursor..];
    if records.len() != metric_count as usize * RECORD_LEN {
        return Err(DecodeError::BadRecordLength);
    }

    Ok(EdgeMetricsFrame {
        seq,
        flags,
        device_id,
        node_name,
        metric_count,
        records,
    })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_metric_frame_without_allocation() {
        let metrics = [
            EdgeMetricSample {
                metric_id: MetricId::EDGE_TEMPERATURE,
                sensor_id: SensorId::ENCLOSURE,
                kind: MetricKind::Gauge,
                unit: Unit::Celsius,
                value: 38.5,
            },
            EdgeMetricSample {
                metric_id: MetricId::EDGE_WATCHDOG_RESETS,
                sensor_id: SensorId::RUNTIME,
                kind: MetricKind::Sum,
                unit: Unit::Count,
                value: 0.0,
            },
        ];
        let identity = EdgeIdentity {
            device_id: "esp32-a",
            node_name: "rack-a",
        };
        let mut out = [0_u8; 96];

        let len = encode_metrics_frame(&mut out, 7, identity, &metrics).unwrap();
        let frame = decode_metrics_frame(&out[..len]).unwrap();

        assert_eq!(frame.seq(), 7);
        assert_eq!(frame.device_id(), "esp32-a");
        assert_eq!(frame.node_name(), "rack-a");
        assert_eq!(frame.metric_count(), 2);

        let mut records = frame.records();
        assert_eq!(records.next().unwrap().unwrap(), metrics[0]);
        assert_eq!(records.next().unwrap().unwrap(), metrics[1]);
        assert!(records.next().is_none());
    }

    #[test]
    fn rejects_truncated_records() {
        let input = [
            MAGIC[0],
            MAGIC[1],
            VERSION,
            FRAME_KIND_METRICS,
            0,
            0,
            1,
            1,
            0,
            0,
            0,
        ];

        assert_eq!(
            decode_metrics_frame(&input).unwrap_err(),
            DecodeError::BadRecordLength
        );
    }
}
