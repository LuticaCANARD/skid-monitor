use super::IngressMessage;
use js_sys::{ArrayBuffer, Uint8Array};
use skid_protocol::{frame, protocol::Signal};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, Event, MessageEvent, UrlSearchParams, WebSocket};

type MessageQueue = Rc<RefCell<VecDeque<IngressMessage>>>;
type ActiveConnections = Rc<RefCell<BTreeSet<String>>>;
type Connections = Rc<RefCell<BTreeMap<String, WebSocketConnection>>>;

pub(crate) struct Ingress {
    messages: MessageQueue,
    control: IngressControl,
}

#[derive(Clone)]
pub(crate) struct IngressControl {
    messages: MessageQueue,
    connections: Connections,
    active: ActiveConnections,
    ctx: eframe::egui::Context,
}

struct WebSocketConnection {
    socket: WebSocket,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl Ingress {
    pub(crate) fn start(ctx: &eframe::egui::Context) -> Self {
        let messages = Rc::new(RefCell::new(VecDeque::new()));
        messages
            .borrow_mut()
            .push_back(IngressMessage::Listening(Vec::new()));
        let control = IngressControl {
            messages: Rc::clone(&messages),
            connections: Rc::new(RefCell::new(BTreeMap::new())),
            active: Rc::new(RefCell::new(BTreeSet::new())),
            ctx: ctx.clone(),
        };

        if let Some(endpoint) = configured_endpoint() {
            if let Err(error) = control.add(endpoint.clone()) {
                push_message(
                    &messages,
                    ctx,
                    IngressMessage::Error {
                        listener: Some(endpoint),
                        error,
                    },
                );
            }
        }

        Self { messages, control }
    }

    pub(crate) fn control(&self) -> IngressControl {
        self.control.clone()
    }

    pub(crate) fn try_next(&mut self) -> Option<IngressMessage> {
        self.messages.borrow_mut().pop_front()
    }
}

impl IngressControl {
    pub(crate) fn add(&self, endpoint: String) -> Result<(), String> {
        let endpoint = normalize_endpoint(&endpoint)?;
        let stale_connection = self
            .connections
            .borrow()
            .get(&endpoint)
            .is_some_and(|connection| connection.socket.ready_state() == WebSocket::CLOSED);
        if stale_connection {
            self.connections.borrow_mut().remove(&endpoint);
        }
        if self.connections.borrow().contains_key(&endpoint) {
            return Err(format!(
                "ingress {endpoint} is already connected or connecting"
            ));
        }

        let socket = WebSocket::new(&endpoint)
            .map_err(|error| format!("failed to open WebSocket {endpoint}: {error:?}"))?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let on_open = {
            let endpoint = endpoint.clone();
            let active = Rc::clone(&self.active);
            let messages = Rc::clone(&self.messages);
            let ctx = self.ctx.clone();
            Closure::wrap(Box::new(move |_event: Event| {
                active.borrow_mut().insert(endpoint.clone());
                publish_active_snapshot(&active, &messages, &ctx);
            }) as Box<dyn FnMut(Event)>)
        };

        let on_message = {
            let endpoint = endpoint.clone();
            let messages = Rc::clone(&self.messages);
            let ctx = self.ctx.clone();
            Closure::wrap(Box::new(move |event: MessageEvent| {
                let message = match decode_message(&event) {
                    Ok(signal) => IngressMessage::Signal {
                        listener: endpoint.clone(),
                        signal,
                    },
                    Err(error) => IngressMessage::Error {
                        listener: Some(endpoint.clone()),
                        error,
                    },
                };
                push_message(&messages, &ctx, message);
            }) as Box<dyn FnMut(MessageEvent)>)
        };

        let on_error = {
            let endpoint = endpoint.clone();
            let messages = Rc::clone(&self.messages);
            let ctx = self.ctx.clone();
            Closure::wrap(Box::new(move |_event: Event| {
                push_message(
                    &messages,
                    &ctx,
                    IngressMessage::Error {
                        listener: Some(endpoint.clone()),
                        error: format!("WebSocket error on {endpoint}"),
                    },
                );
            }) as Box<dyn FnMut(Event)>)
        };

        let on_close = {
            let endpoint = endpoint.clone();
            let active = Rc::clone(&self.active);
            let messages = Rc::clone(&self.messages);
            let ctx = self.ctx.clone();
            Closure::wrap(Box::new(move |event: CloseEvent| {
                active.borrow_mut().remove(&endpoint);
                publish_active_snapshot(&active, &messages, &ctx);
                if !event.was_clean() {
                    push_message(
                        &messages,
                        &ctx,
                        IngressMessage::Error {
                            listener: Some(endpoint.clone()),
                            error: format!(
                                "WebSocket {endpoint} closed with code {}: {}",
                                event.code(),
                                event.reason()
                            ),
                        },
                    );
                }
            }) as Box<dyn FnMut(CloseEvent)>)
        };

        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        self.connections.borrow_mut().insert(
            endpoint,
            WebSocketConnection {
                socket,
                _on_open: on_open,
                _on_message: on_message,
                _on_error: on_error,
                _on_close: on_close,
            },
        );
        Ok(())
    }

    pub(crate) fn remove(&self, endpoint: String) -> Result<(), String> {
        let endpoint = normalize_endpoint(&endpoint)?;
        let Some(connection) = self.connections.borrow_mut().remove(&endpoint) else {
            return Err(format!("ingress {endpoint} is not connected"));
        };

        connection.socket.set_onopen(None);
        connection.socket.set_onmessage(None);
        connection.socket.set_onerror(None);
        connection.socket.set_onclose(None);
        self.active.borrow_mut().remove(&endpoint);
        publish_active_snapshot(&self.active, &self.messages, &self.ctx);
        connection
            .socket
            .close()
            .map_err(|error| format!("failed to close WebSocket {endpoint}: {error:?}"))
    }
}

fn normalize_endpoint(endpoint: &str) -> Result<String, String> {
    let endpoint = endpoint.trim();
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        Ok(endpoint.to_string())
    } else {
        Err("web ingress must use a ws:// or wss:// URL".to_string())
    }
}

fn configured_endpoint() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = UrlSearchParams::new_with_str(&search).ok()?;
    params
        .get("ingress")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn decode_message(event: &MessageEvent) -> Result<Signal, String> {
    let data = event.data();
    if let Some(text) = data.as_string() {
        return serde_json::from_str(&text)
            .map_err(|error| format!("invalid WebSocket signal JSON: {error}"));
    }
    if data.is_instance_of::<ArrayBuffer>() {
        let bytes = Uint8Array::new(&data).to_vec();
        return frame::decode_signal_payload(&bytes)
            .map_err(|error| format!("invalid WebSocket signal payload: {error}"));
    }
    Err("unsupported WebSocket message type; expected JSON text or ArrayBuffer".to_string())
}

fn publish_active_snapshot(
    active: &ActiveConnections,
    messages: &MessageQueue,
    ctx: &eframe::egui::Context,
) {
    let endpoints = active.borrow().iter().cloned().collect();
    push_message(messages, ctx, IngressMessage::Listening(endpoints));
}

fn push_message(messages: &MessageQueue, ctx: &eframe::egui::Context, message: IngressMessage) {
    messages.borrow_mut().push_back(message);
    ctx.request_repaint();
}
