//! Shared client-side signal receiver loop.
//!
//! The low-level TCP bind/read path stays in [`crate::receiver`]. This module
//! wraps that binder with extension delivery and an app-friendly channel so GUI
//! and TUI clients can share the same receive semantics.

use crate::extension::ExtensionHost;
use crate::receiver::{Receiver as SignalReceiver, listen_addrs};
use skid_protocol::protocol::Signal;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

/// Messages emitted by the shared receiver loop.
pub enum ReceiverMessage {
    Listening(Vec<String>),
    Signal { listener: String, signal: Signal },
    Error(String),
    ExtensionError(String),
}

/// Starts the shared receiver loop on a background thread.
///
/// The loop binds the configured client address list, receives one length-prefixed
/// `Signal` per TCP connection, forwards signals to the optional extension
/// host, and emits app-facing status/messages over a channel.
pub fn spawn_receiver() -> Receiver<ReceiverMessage> {
    spawn_receiver_on(listen_addrs())
}

/// Starts the shared receiver loop on explicit listen addresses.
///
/// This is useful for tests and for clients that have already resolved their
/// own configuration. Production callers normally use [`spawn_receiver`].
pub fn spawn_receiver_on(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, true)
}

#[cfg(test)]
pub(crate) fn spawn_receiver_on_without_extension(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, false)
}

fn spawn_receiver_configured(
    addrs: Vec<String>,
    start_extension: bool,
) -> Receiver<ReceiverMessage> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut receivers = Vec::new();
        for configured_addr in addrs {
            match SignalReceiver::bind(&configured_addr) {
                Ok(receiver) => {
                    let listen_addr = receiver
                        .local_addr()
                        .map(|addr| addr.to_string())
                        .unwrap_or(configured_addr);
                    receivers.push((listen_addr, receiver));
                }
                Err(err) => {
                    let _ = tx.send(ReceiverMessage::Error(format!(
                        "failed to bind {configured_addr}: {err}"
                    )));
                }
            }
        }

        if receivers.is_empty() {
            let _ = tx.send(ReceiverMessage::Error(
                "failed to bind any configured client listener".to_string(),
            ));
            return;
        }

        let active_addrs = receivers
            .iter()
            .map(|(addr, _)| addr.clone())
            .collect::<Vec<_>>();
        if tx.send(ReceiverMessage::Listening(active_addrs)).is_err() {
            return;
        }

        let extension = if start_extension {
            match ExtensionHost::from_env() {
                Ok(host) => host,
                Err(err) => {
                    let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                        "failed to start extension host: {err}"
                    )));
                    None
                }
            }
        } else {
            None
        };
        let extension = Arc::new(Mutex::new(extension));

        for (addr, receiver) in receivers {
            let tx = tx.clone();
            let extension = Arc::clone(&extension);
            thread::spawn(move || receive_forever(addr, receiver, tx, extension));
        }
    });
    rx
}

fn receive_forever(
    addr: String,
    receiver: SignalReceiver,
    tx: Sender<ReceiverMessage>,
    extension: Arc<Mutex<Option<ExtensionHost>>>,
) {
    loop {
        match receiver.recv() {
            Ok(signal) => {
                publish_to_extension(&signal, &tx, &extension);
                if tx
                    .send(ReceiverMessage::Signal {
                        listener: addr.clone(),
                        signal,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(err) => {
                if tx
                    .send(ReceiverMessage::Error(format!(
                        "receive error on {addr}: {err}"
                    )))
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

fn publish_to_extension(
    signal: &Signal,
    tx: &Sender<ReceiverMessage>,
    extension: &Arc<Mutex<Option<ExtensionHost>>>,
) {
    match extension.lock() {
        Ok(mut guard) => {
            if let Some(extension) = guard.as_mut() {
                if let Err(err) = extension.publish_signal(signal) {
                    let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                        "failed to publish to extension host: {err}"
                    )));
                }
            }
        }
        Err(err) => {
            let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                "extension host lock poisoned: {err}"
            )));
        }
    }
}
