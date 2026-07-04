use super::*;
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};

#[test]
fn reads_length_prefixed_signal() {
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
