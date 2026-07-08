//! Shared client-side signal receiver loop.
//!
//! The low-level TCP bind/read path stays in [`crate::receiver`]. This module
//! wraps that binder with extension delivery and an app-friendly channel so GUI
//! and TUI clients can share the same receive semantics.

use crate::extension::ExtensionHost;
use crate::receiver::{Receiver as SignalReceiver, listen_addrs};
use skid_protocol::protocol::Signal;
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type NotifyReceiverUpdate = Arc<dyn Fn() + Send + Sync + 'static>;
/// Per-listener shutdown flags, keyed by resolved listen address, so a
/// `RemoveListener` request can find and stop the right `receive_forever`
/// thread.
type ListenerRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// Messages emitted by the shared receiver loop.
pub enum ReceiverMessage {
    /// Full snapshot of currently active client ingress listeners.
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

/// Requests a caller can send back into a running receiver loop.
pub enum ReceiverControl {
    /// Bind an additional client ingress address at runtime, without restarting
    /// the process. Success/failure is reported back over the existing
    /// `ReceiverMessage` channel (`Listening`/`Error`), same as startup binds.
    AddListener(String),
    /// Stop and unbind a client ingress listener that is currently running
    /// (whether it came from the initial configured batch or from
    /// `AddListener`), without restarting the process.
    RemoveListener(String),
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
    spawn_receiver_configured(listen_addrs(), true, Some(Arc::new(notify))).0
}

/// Like [`spawn_receiver_with_notify`], but also returns a control channel
/// that lets callers manage client ingress listeners at runtime, without
/// restarting the process.
pub fn spawn_receiver_managed_with_notify(
    notify: impl Fn() + Send + Sync + 'static,
) -> (Receiver<ReceiverMessage>, Sender<ReceiverControl>) {
    spawn_receiver_configured(listen_addrs(), true, Some(Arc::new(notify)))
}

/// Starts the shared receiver loop on explicit listen addresses.
///
/// This is useful for tests and for clients that have already resolved their
/// own configuration. Production callers normally use [`spawn_receiver`].
pub fn spawn_receiver_on(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, true, None).0
}

#[cfg(test)]
pub(crate) fn spawn_receiver_on_without_extension(addrs: Vec<String>) -> Receiver<ReceiverMessage> {
    spawn_receiver_configured(addrs, false, None).0
}

#[cfg(test)]
pub(crate) fn spawn_receiver_managed_on_without_extension(
    addrs: Vec<String>,
) -> (Receiver<ReceiverMessage>, Sender<ReceiverControl>) {
    spawn_receiver_configured(addrs, false, None)
}

fn spawn_receiver_configured(
    addrs: Vec<String>,
    start_extension: bool,
    notify: Option<NotifyReceiverUpdate>,
) -> (Receiver<ReceiverMessage>, Sender<ReceiverControl>) {
    let (tx, rx) = mpsc::channel();
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<ReceiverControl>();

    thread::spawn(move || {
        let registry: ListenerRegistry = Arc::new(Mutex::new(HashMap::new()));
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

        if !receivers.is_empty() {
            for (addr, receiver) in receivers {
                spawn_listener(&registry, addr, receiver, &tx, &extension, &notify);
            }
            if !send_listener_snapshot(&registry, &tx, notify.as_ref()) {
                return;
            }
        }

        // Stay alive after the initial batch so runtime listener requests can
        // (un)bind sockets without restarting the process.
        while let Ok(control) = ctrl_rx.recv() {
            match control {
                ReceiverControl::AddListener(addr) => match SignalReceiver::bind(&addr) {
                    Ok(receiver) => {
                        let listen_addr = receiver
                            .local_addr()
                            .map(|resolved| resolved.to_string())
                            .unwrap_or_else(|_| addr.clone());
                        spawn_listener(&registry, listen_addr, receiver, &tx, &extension, &notify);
                        if !send_listener_snapshot(&registry, &tx, notify.as_ref()) {
                            break;
                        }
                    }
                    Err(err) => {
                        send_message(
                            &tx,
                            notify.as_ref(),
                            ReceiverMessage::Error {
                                listener: Some(addr.clone()),
                                error: format!("failed to bind {addr}: {err}"),
                            },
                        );
                    }
                },
                ReceiverControl::RemoveListener(addr) => {
                    if stop_listener(&registry, &addr) {
                        if !send_listener_snapshot(&registry, &tx, notify.as_ref()) {
                            break;
                        }
                    } else {
                        send_message(
                            &tx,
                            notify.as_ref(),
                            ReceiverMessage::Error {
                                listener: Some(addr.clone()),
                                error: format!("listener {addr} is not active"),
                            },
                        );
                    }
                }
            }
        }
    });

    (rx, ctrl_tx)
}

/// Spawns the receive thread for one bound listener and registers its stop
/// flag so a later `RemoveListener` request can find and signal it.
fn spawn_listener(
    registry: &ListenerRegistry,
    addr: String,
    receiver: SignalReceiver,
    tx: &Sender<ReceiverMessage>,
    extension: &Arc<Mutex<Option<ExtensionHost>>>,
    notify: &Option<NotifyReceiverUpdate>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut listeners) = registry.lock() {
        listeners.insert(addr.clone(), Arc::clone(&stop));
    }

    let tx = tx.clone();
    let extension = Arc::clone(extension);
    let notify = notify.clone();
    thread::spawn(move || receive_forever(addr, receiver, tx, extension, notify, stop));
}

/// Signals a running listener's thread to stop and unblocks its pending
/// `accept()` call so the thread can actually observe the flag and exit.
fn stop_listener(registry: &ListenerRegistry, addr: &str) -> bool {
    let Some(stop) = registry
        .lock()
        .ok()
        .and_then(|mut listeners| listeners.remove(addr))
    else {
        return false;
    };
    stop.store(true, Ordering::Relaxed);

    // `TcpListener::accept` blocks indefinitely; open a throwaway local
    // connection to wake it up so the thread notices `stop` and exits.
    if let Ok(addr) = addr.parse() {
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(200));
    }
    true
}

fn receive_forever(
    addr: String,
    receiver: SignalReceiver,
    tx: Sender<ReceiverMessage>,
    extension: Arc<Mutex<Option<ExtensionHost>>>,
    notify: Option<NotifyReceiverUpdate>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match receiver.recv() {
            Ok(signal) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
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
            Err(_) if stop.load(Ordering::Relaxed) => break,
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

fn send_listener_snapshot(
    registry: &ListenerRegistry,
    tx: &Sender<ReceiverMessage>,
    notify: Option<&NotifyReceiverUpdate>,
) -> bool {
    send_message(
        tx,
        notify,
        ReceiverMessage::Listening(active_listeners(registry)),
    )
}

fn active_listeners(registry: &ListenerRegistry) -> Vec<String> {
    let mut addrs = registry
        .lock()
        .map(|listeners| listeners.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    addrs.sort();
    addrs
}
