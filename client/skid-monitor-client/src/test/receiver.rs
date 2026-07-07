use super::*;
use crate::receiver_loop::{ReceiverMessage, spawn_receiver_on_without_extension};
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};
use std::net::TcpStream;
use std::time::Duration;

fn sample_signal(name: &str, value: f64) -> Signal {
    Signal::Metrics(export_metrics(
        vec![Metric {
            name: name.to_string(),
            value,
            source: Source::System,
            unit: Some("%".to_string()),
            kind: MetricKind::Gauge,
            attributes: vec![("host".to_string(), "local".to_string())],
        }],
        "test-service",
        "test-scope",
    ))
}

#[test]
fn reads_length_prefixed_signal() {
    let signal = sample_signal("cpu.usage", 42.0);
    let mut frame = Vec::new();
    skid_protocol::frame::write_signal(&mut frame, &signal).unwrap();

    let decoded = skid_protocol::frame::read_signal(&mut frame.as_slice()).unwrap();
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
    let frame = (skid_protocol::frame::LEGACY_MAX_FRAME_BYTES + 1)
        .to_be_bytes()
        .to_vec();
    let result = skid_protocol::frame::read_signal(&mut frame.as_slice());
    assert!(result.is_err());
    let err = result.err().unwrap();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn parses_multiple_client_listen_addrs() {
    let addrs = parse_addr_list(" 127.0.0.1:9000,127.0.0.1:9001,,127.0.0.1:9000, 127.0.0.1:9002 ");

    assert_eq!(
        addrs,
        vec![
            "127.0.0.1:9000".to_string(),
            "127.0.0.1:9001".to_string(),
            "127.0.0.1:9002".to_string(),
        ]
    );
}

#[test]
fn receiver_loop_accepts_signals_on_multiple_listeners() {
    let rx = spawn_receiver_on_without_extension(vec![
        "127.0.0.1:0".to_string(),
        "127.0.0.1:0".to_string(),
    ]);

    let addrs = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => addrs,
        ReceiverMessage::Error(error) => panic!("receiver failed to bind: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived before listener status"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    };
    assert_eq!(addrs.len(), 2);

    for (index, addr) in addrs.iter().enumerate() {
        let signal = sample_signal("cpu.usage", 40.0 + index as f64);
        let mut stream = TcpStream::connect(addr).unwrap();
        skid_protocol::frame::write_signal(&mut stream, &signal).unwrap();
    }

    let mut received = 0;
    while received < 2 {
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            ReceiverMessage::Signal {
                listener,
                signal: Signal::Metrics(_),
            } => {
                assert!(addrs.iter().any(|addr| addr == &listener));
                received += 1;
            }
            ReceiverMessage::Listening(addrs) => panic!("duplicate listener status: {addrs:?}"),
            ReceiverMessage::Error(error) => panic!("receive failed: {error}"),
            ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
            ReceiverMessage::Signal { .. } => panic!("unexpected signal kind"),
        }
    }
}
