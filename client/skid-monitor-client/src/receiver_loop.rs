//! Shared client-side signal receiver loop.
//!
//! The low-level TCP bind/read path stays in [`crate::receiver`]. This module
//! wraps that binder with extension delivery and an app-friendly channel so GUI
//! and TUI clients can share the same receive semantics.

use crate::extension::ExtensionHost;
use crate::receiver::{Receiver as SignalReceiver, listen_addr};
use skid_protocol::protocol::Signal;
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// Messages emitted by the shared receiver loop.
pub enum ReceiverMessage {
    Listening(String),
    Signal(Signal),
    Error(String),
    ExtensionError(String),
}

/// Starts the shared receiver loop on a background thread.
///
/// The loop binds the configured client address, receives one length-prefixed
/// `Signal` per TCP connection, forwards signals to the optional extension
/// host, and emits app-facing status/messages over a channel.
pub fn spawn_receiver() -> Receiver<ReceiverMessage> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let addr = listen_addr();
        let mut extension = match ExtensionHost::from_env() {
            Ok(host) => host,
            Err(err) => {
                let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                    "failed to start extension host: {err}"
                )));
                None
            }
        };

        let receiver = match SignalReceiver::bind(&addr) {
            Ok(receiver) => receiver,
            Err(err) => {
                let _ = tx.send(ReceiverMessage::Error(format!(
                    "failed to bind {addr}: {err}"
                )));
                return;
            }
        };

        if tx.send(ReceiverMessage::Listening(addr)).is_err() {
            return;
        }

        loop {
            match receiver.recv() {
                Ok(signal) => {
                    if let Some(extension) = extension.as_mut() {
                        if let Err(err) = extension.publish_signal(&signal) {
                            let _ = tx.send(ReceiverMessage::ExtensionError(format!(
                                "failed to publish to extension host: {err}"
                            )));
                        }
                    }
                    if tx.send(ReceiverMessage::Signal(signal)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    if tx
                        .send(ReceiverMessage::Error(format!("receive error: {err}")))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    rx
}
