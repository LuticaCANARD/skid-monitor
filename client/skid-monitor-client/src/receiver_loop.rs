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

type NotifyReceiverUpdate = Arc<dyn Fn() + Send + Sync + 'static>;

/// Messages emitted by the shared receiver loop.
pub enum ReceiverMessage {
    Listening(Vec<String>),
    Signal {
        listener: String,
        signal: Signal,
    },
    Error {
        listener: Option<String>,
        error: String,
    },
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

/// Starts the shared receiver loop and calls `notify` after each emitted message.
pub fn spawn_receiver_with_notify(
    notify: impl Fn() + Send + Sync + 'static,
) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(listen_addrs(), true, Some(Arc::new(notify)))
}

/// Starts the shared receiver loop on explicit listen addresses.
///
/// This is useful for tests and for clients that have already resolved their
/// own configuration. Production callers normally use [`spawn_receiver`].
pub fn spawn_receiver_on(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, true, None)
}

#[cfg(test)]
pub(crate) fn spawn_receiver_on_without_extension(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, false, None)
}

fn spawn_receiver_configured(
    addrs: Vec<String>,
    start_extension: bool,
    notify: Option<NotifyReceiverUpdate>,
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
                    send_message(
                        &tx,
                        notify.as_ref(),
                        ReceiverMessage::Error {
                            listener: Some(configured_addr.clone()),
                            error: format!("failed to bind {configured_addr}: {err}"),
                        },
                    );
                }
            }
        }

        if receivers.is_empty() {
            send_message(
                &tx,
                notify.as_ref(),
                ReceiverMessage::Error {
                    listener: None,
                    error: "failed to bind any configured client listener".to_string(),
                },
            );
            return;
        }

        let active_addrs = receivers
            .iter()
            .map(|(addr, _)| addr.clone())
            .collect::<Vec<_>>();
        if !send_message(
            &tx,
            notify.as_ref(),
            ReceiverMessage::Listening(active_addrs),
        ) {
            return;
        }

        let extension = if start_extension {
            match ExtensionHost::from_env() {
                Ok(host) => host,
                Err(err) => {
                    send_message(
                        &tx,
                        notify.as_ref(),
                        ReceiverMessage::ExtensionError(format!(
                            "failed to start extension host: {err}"
                        )),
                    );
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
            let notify = notify.clone();
            thread::spawn(move || receive_forever(addr, receiver, tx, extension, notify));
        }
    });
    rx
}

fn receive_forever(
    addr: String,
    receiver: SignalReceiver,
    tx: Sender<ReceiverMessage>,
    extension: Arc<Mutex<Option<ExtensionHost>>>,
    notify: Option<NotifyReceiverUpdate>,
) {
    loop {
        match receiver.recv() {
            Ok(signal) => {
                publish_to_extension(&signal, &tx, &extension, notify.as_ref());
                if !send_message(
                    &tx,
                    notify.as_ref(),
                    ReceiverMessage::Signal {
                        listener: addr.clone(),
                        signal,
                    },
                ) {
                    break;
                }
            }
            Err(err) => {
                if !send_message(
                    &tx,
                    notify.as_ref(),
                    ReceiverMessage::Error {
                        listener: Some(addr.clone()),
                        error: format!("receive error on {addr}: {err}"),
                    },
                ) {
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
    notify: Option<&NotifyReceiverUpdate>,
) {
    match extension.lock() {
        Ok(mut guard) => {
            if let Some(extension) = guard.as_mut() {
                if let Err(err) = extension.publish_signal(signal) {
                    send_message(
                        tx,
                        notify,
                        ReceiverMessage::ExtensionError(format!(
                            "failed to publish to extension host: {err}"
                        )),
                    );
                }
            }
        }
        Err(err) => {
            send_message(
                tx,
                notify,
                ReceiverMessage::ExtensionError(format!("extension host lock poisoned: {err}")),
            );
        }
    }
}

fn send_message(
    tx: &Sender<ReceiverMessage>,
    notify: Option<&NotifyReceiverUpdate>,
    message: ReceiverMessage,
) -> bool {
    if tx.send(message).is_err() {
        return false;
    }
    if let Some(notify) = notify {
        notify();
    }
    true
}
