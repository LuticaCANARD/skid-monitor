use super::*;
use crate::receiver_loop::{
    ReceiverControl, ReceiverMessage, spawn_receiver_managed_on_without_extension,
    spawn_receiver_on_without_extension, spawn_solo_receiver_managed_on_without_extension,
};
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

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
fn trusted_local_address_validation_accepts_only_numeric_loopback_addresses() {
    let ipv4 = trusted_local_socket_addr("127.0.0.1:9000").unwrap();
    let ipv6 = trusted_local_socket_addr("[::1]:9000").unwrap();

    assert!(ipv4.ip().is_loopback());
    assert!(ipv6.ip().is_loopback());

    for rejected in [
        "0.0.0.0:9000",
        "[::]:9000",
        "192.0.2.10:9000",
        "[2001:db8::10]:9000",
        "localhost:9000",
        "example.invalid:9000",
    ] {
        let err = trusted_local_socket_addr(rejected).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{rejected}");
        assert!(
            err.to_string().contains("trusted-local listener"),
            "unexpected error for {rejected}: {err}"
        );
    }
}

#[test]
fn trusted_local_receiver_binds_an_ephemeral_loopback_port() {
    let receiver = Receiver::bind_trusted_local("127.0.0.1:0").unwrap();

    assert!(receiver.local_addr().unwrap().ip().is_loopback());
}

#[test]
fn partial_frame_read_times_out_and_listener_accepts_the_next_signal() {
    let receiver =
        Receiver::bind_trusted_local_with_read_timeout("127.0.0.1:0", Duration::from_millis(75))
            .unwrap();
    let addr = receiver.local_addr().unwrap();
    let mut stalled = TcpStream::connect(addr).unwrap();
    stalled.write_all(&[0, 0]).unwrap();

    let started = Instant::now();
    let err = match receiver.recv() {
        Ok(_) => panic!("partial frame unexpectedly decoded"),
        Err(err) => err,
    };

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "partial frame exceeded the bounded read deadline"
    );

    drop(stalled);
    let signal = sample_signal("cpu.usage", 42.0);
    let mut next = TcpStream::connect(addr).unwrap();
    skid_protocol::frame::write_signal(&mut next, &signal).unwrap();

    assert!(matches!(receiver.recv().unwrap(), Signal::Metrics(_)));
}

#[test]
fn unrestricted_receiver_bind_remains_compatible_with_wildcard_addresses() {
    let receiver = Receiver::bind("0.0.0.0:0").unwrap();

    assert!(receiver.local_addr().unwrap().ip().is_unspecified());
}

#[test]
fn receiver_loop_accepts_signals_on_multiple_listeners() {
    let rx = spawn_receiver_on_without_extension(vec![
        "127.0.0.1:0".to_string(),
        "127.0.0.1:0".to_string(),
    ]);

    let addrs = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => addrs,
        ReceiverMessage::Error { error, .. } => panic!("receiver failed to bind: {error}"),
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
            ReceiverMessage::Error { error, .. } => panic!("receive failed: {error}"),
            ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
            ReceiverMessage::Signal { .. } => panic!("unexpected signal kind"),
        }
    }
}

#[test]
fn remove_listener_frees_the_bound_port() {
    let (rx, ctrl_tx) =
        spawn_receiver_managed_on_without_extension(vec!["127.0.0.1:0".to_string()]);

    let addr = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => addrs.into_iter().next().unwrap(),
        other => panic!(
            "expected Listening, got a different message: {}",
            match other {
                ReceiverMessage::Error { error, .. } => error,
                ReceiverMessage::ExtensionError(error) => error,
                _ => "signal".to_string(),
            }
        ),
    };

    ctrl_tx
        .send(ReceiverControl::RemoveListener(addr.clone()))
        .unwrap();

    match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => assert!(addrs.is_empty()),
        ReceiverMessage::Error { error, .. } => panic!("remove listener failed: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived while removing listener"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    }

    // The receiver loop only notices `stop` after a wakeup connect unblocks
    // its pending `accept()`; poll until the OS actually lets us rebind the
    // freed port instead of asserting on a fixed sleep.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if TcpListener::bind(&addr).is_ok() {
            break;
        }
        if Instant::now() > deadline {
            panic!("listener on {addr} was not released after RemoveListener");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn add_listener_reports_the_active_listener_snapshot() {
    let (rx, ctrl_tx) =
        spawn_receiver_managed_on_without_extension(vec!["127.0.0.1:0".to_string()]);

    let first_addr = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => {
            assert_eq!(addrs.len(), 1);
            addrs.into_iter().next().unwrap()
        }
        ReceiverMessage::Error { error, .. } => panic!("receiver failed to bind: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived before listener status"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    };

    ctrl_tx
        .send(ReceiverControl::AddListener("127.0.0.1:0".to_string()))
        .unwrap();

    let addrs = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => addrs,
        ReceiverMessage::Error { error, .. } => panic!("add listener failed: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived while adding listener"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    };

    assert_eq!(addrs.len(), 2);
    assert!(addrs.contains(&first_addr));
}

#[test]
fn solo_receiver_rejects_untrusted_startup_addresses_and_binds_loopback() {
    let rejected = ["0.0.0.0:0", "localhost:0", "192.0.2.10:0"];
    let mut configured = rejected
        .iter()
        .map(|addr| (*addr).to_string())
        .collect::<Vec<_>>();
    configured.push("127.0.0.1:0".to_string());

    let (rx, _ctrl_tx) = spawn_solo_receiver_managed_on_without_extension(configured);

    for expected_listener in rejected {
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            ReceiverMessage::Error {
                listener: Some(listener),
                error,
            } => {
                assert_eq!(listener, expected_listener);
                assert!(error.contains("trusted-local listener"), "{error}");
            }
            ReceiverMessage::Error {
                listener: None,
                error,
            } => {
                panic!("unexpected receiver-wide startup error: {error}")
            }
            ReceiverMessage::Listening(addrs) => {
                panic!("listener snapshot arrived before startup rejection: {addrs:?}")
            }
            ReceiverMessage::Signal { .. } => panic!("signal arrived during startup"),
            ReceiverMessage::ExtensionError(error) => {
                panic!("unexpected extension error: {error}")
            }
        }
    }

    match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => {
            assert_eq!(addrs.len(), 1);
            let addr: std::net::SocketAddr = addrs[0].parse().unwrap();
            assert!(addr.ip().is_loopback());
        }
        ReceiverMessage::Error { error, .. } => panic!("loopback bind failed: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived before listener status"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    }
}

#[test]
fn solo_receiver_enforces_trusted_local_policy_for_runtime_add_listener() {
    let (rx, ctrl_tx) =
        spawn_solo_receiver_managed_on_without_extension(vec!["127.0.0.1:0".to_string()]);

    let first_addr = match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => {
            assert_eq!(addrs.len(), 1);
            addrs.into_iter().next().unwrap()
        }
        ReceiverMessage::Error { error, .. } => panic!("receiver failed to bind: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived before listener status"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    };

    for rejected in ["0.0.0.0:0", "[::]:0", "localhost:0", "192.0.2.10:0"] {
        ctrl_tx
            .send(ReceiverControl::AddListener(rejected.to_string()))
            .unwrap();

        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            ReceiverMessage::Error {
                listener: Some(listener),
                error,
            } => {
                assert_eq!(listener, rejected);
                assert!(error.contains("trusted-local listener"), "{error}");
            }
            ReceiverMessage::Error {
                listener: None,
                error,
            } => {
                panic!("unexpected receiver-wide error: {error}")
            }
            ReceiverMessage::Listening(addrs) => {
                panic!("rejected runtime bind changed listener snapshot: {addrs:?}")
            }
            ReceiverMessage::Signal { .. } => panic!("signal arrived while adding listener"),
            ReceiverMessage::ExtensionError(error) => {
                panic!("unexpected extension error: {error}")
            }
        }
    }

    ctrl_tx
        .send(ReceiverControl::AddListener("127.0.0.1:0".to_string()))
        .unwrap();

    match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
        ReceiverMessage::Listening(addrs) => {
            assert_eq!(addrs.len(), 2);
            assert!(addrs.contains(&first_addr));
            assert!(addrs.iter().all(|addr| {
                addr.parse::<std::net::SocketAddr>()
                    .map(|addr| addr.ip().is_loopback())
                    .unwrap_or(false)
            }));
        }
        ReceiverMessage::Error { error, .. } => panic!("loopback add failed: {error}"),
        ReceiverMessage::Signal { .. } => panic!("signal arrived while adding listener"),
        ReceiverMessage::ExtensionError(error) => panic!("unexpected extension error: {error}"),
    }
}
