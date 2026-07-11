use super::cloud::{
    ACCESS_TOKEN_SESSION_KEY, BrowserEndpoint, DecodedJsonSignal, cursor_storage_key,
    decode_json_signal, normalize_endpoint, stream_endpoint, tenant_from_stream_ticket,
    ticket_endpoint,
};
use super::{BrowserStorageScope, IngressMessage};
use gloo_timers::future::TimeoutFuture;
use js_sys::{ArrayBuffer, Uint8Array};
use serde::Deserialize;
use skid_monitor_core::TenantId;
use skid_protocol::{frame, protocol::Signal};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use url::Url;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    CloseEvent, Event, Headers, MessageEvent, Request, RequestCache, RequestInit, RequestMode,
    Response, UrlSearchParams, WebSocket,
};

type MessageQueue = Rc<RefCell<VecDeque<IngressMessage>>>;
type ActiveConnections = Rc<RefCell<BTreeSet<String>>>;
type Connections = Rc<RefCell<BTreeMap<String, BrowserConnection>>>;

const MAX_RECONNECT_ATTEMPTS: u8 = 8;
const MAX_RECONNECT_DELAY_MS: u32 = 30_000;
const MAX_TICKET_BYTES: usize = 4_096;
const MAX_TICKET_RESPONSE_BYTES: usize = 8_192;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1_024;

pub(crate) struct Ingress {
    messages: MessageQueue,
    control: IngressControl,
}

#[derive(Clone)]
pub(crate) struct IngressControl {
    messages: MessageQueue,
    connections: Connections,
    active: ActiveConnections,
    next_generation: Rc<Cell<u64>>,
    ctx: eframe::egui::Context,
}

enum BrowserConnection {
    Raw(WebSocketConnection),
    Cloud(CloudConnection),
}

struct CloudConnection {
    generation: u64,
    tenant_id: Option<TenantId>,
    socket: Option<WebSocketConnection>,
    reconnect_attempt: u8,
    persisted_cursor: u64,
    connecting: bool,
}

struct WebSocketConnection {
    socket: WebSocket,
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl WebSocketConnection {
    fn detach_handlers(&self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
    }

    fn close(&self) -> Result<(), String> {
        self.socket
            .close()
            .map_err(|_| "failed to close browser ingress WebSocket".to_string())
    }
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
            next_generation: Rc::new(Cell::new(1)),
            ctx: ctx.clone(),
        };

        for endpoint in configured_endpoints() {
            if let Err(error) = control.add(endpoint.clone()) {
                control.report_error(Some(endpoint), error);
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
        let key = endpoint.key().to_string();

        self.connections.borrow_mut().retain(|_, connection| {
            !matches!(connection, BrowserConnection::Raw(raw) if raw.socket.ready_state() == WebSocket::CLOSED)
        });
        if self.connections.borrow().contains_key(&key) {
            return Err(format!("ingress {key} is already connected or connecting"));
        }

        let has_cloud = self
            .connections
            .borrow()
            .values()
            .any(|connection| matches!(connection, BrowserConnection::Cloud(_)));
        let has_raw = self
            .connections
            .borrow()
            .values()
            .any(|connection| matches!(connection, BrowserConnection::Raw(_)));
        match &endpoint {
            BrowserEndpoint::CloudApi(_) if has_cloud || has_raw => {
                return Err(
                    "cloud client API mode must be the browser's only ingress connection"
                        .to_string(),
                );
            }
            BrowserEndpoint::RawWebSocket(_) if has_cloud => {
                return Err(
                    "raw WebSocket ingress cannot be mixed with cloud client API mode".to_string(),
                );
            }
            _ => {}
        }

        match endpoint {
            BrowserEndpoint::RawWebSocket(endpoint) => self.add_raw(endpoint),
            BrowserEndpoint::CloudApi(endpoint) => self.add_cloud(endpoint),
        }
    }

    fn add_raw(&self, endpoint: String) -> Result<(), String> {
        let connection = open_raw_socket(self, &endpoint)?;
        self.connections
            .borrow_mut()
            .insert(endpoint, BrowserConnection::Raw(connection));
        publish_storage_scope(&self.messages, &self.ctx, BrowserStorageScope::Legacy);
        Ok(())
    }

    fn add_cloud(&self, endpoint: String) -> Result<(), String> {
        require_same_origin(&endpoint)?;
        let generation = self.next_generation.get();
        self.next_generation.set(generation.wrapping_add(1).max(1));
        self.connections.borrow_mut().insert(
            endpoint.clone(),
            BrowserConnection::Cloud(CloudConnection {
                generation,
                tenant_id: None,
                socket: None,
                reconnect_attempt: 0,
                persisted_cursor: 0,
                connecting: false,
            }),
        );

        publish_storage_scope(&self.messages, &self.ctx, BrowserStorageScope::CloudPending);
        // Cloud connections remain listed while reconnecting so an explicit
        // Disconnect always cancels pending fetch/backoff work.
        self.active.borrow_mut().insert(endpoint.clone());
        publish_active_snapshot(&self.active, &self.messages, &self.ctx);
        begin_cloud_connect(self.clone(), endpoint, generation);
        Ok(())
    }

    pub(crate) fn remove(&self, endpoint: String) -> Result<(), String> {
        let endpoint = normalize_endpoint(&endpoint)?;
        let key = endpoint.key().to_string();
        let Some(connection) = self.connections.borrow_mut().remove(&key) else {
            return Err(format!("ingress {key} is not connected"));
        };

        let was_cloud = matches!(&connection, BrowserConnection::Cloud(_));
        let socket = match &connection {
            BrowserConnection::Raw(connection) => Some(connection),
            BrowserConnection::Cloud(connection) => connection.socket.as_ref(),
        };
        if let Some(socket) = socket {
            socket.detach_handlers();
        }
        self.active.borrow_mut().remove(&key);
        publish_active_snapshot(&self.active, &self.messages, &self.ctx);

        if was_cloud {
            publish_storage_scope(&self.messages, &self.ctx, BrowserStorageScope::CloudPending);
        }
        if let Some(socket) = socket {
            socket.close()?;
        }
        Ok(())
    }

    fn report_error(&self, listener: Option<String>, error: String) {
        push_message(
            &self.messages,
            &self.ctx,
            IngressMessage::Error { listener, error },
        );
    }
}

fn require_same_origin(endpoint: &str) -> Result<(), String> {
    let page_origin = web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_string())?
        .location()
        .origin()
        .map_err(js_error)?;
    let endpoint_origin = Url::parse(endpoint)
        .map_err(|_| "invalid cloud client API URL".to_string())?
        .origin()
        .ascii_serialization();
    if endpoint_origin != page_origin {
        return Err(
            "cloud client API must share the frontend origin; use a same-origin reverse proxy"
                .to_string(),
        );
    }
    Ok(())
}

fn open_raw_socket(
    control: &IngressControl,
    endpoint: &str,
) -> Result<WebSocketConnection, String> {
    let socket =
        WebSocket::new(endpoint).map_err(|_| format!("failed to open WebSocket {endpoint}"))?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let on_open = {
        let endpoint = endpoint.to_string();
        let active = Rc::clone(&control.active);
        let messages = Rc::clone(&control.messages);
        let ctx = control.ctx.clone();
        Closure::wrap(Box::new(move |_event: Event| {
            active.borrow_mut().insert(endpoint.clone());
            publish_active_snapshot(&active, &messages, &ctx);
        }) as Box<dyn FnMut(Event)>)
    };

    let on_message = {
        let endpoint = endpoint.to_string();
        let messages = Rc::clone(&control.messages);
        let ctx = control.ctx.clone();
        Closure::wrap(Box::new(move |event: MessageEvent| {
            let message = match decode_raw_message(&event) {
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
        let endpoint = endpoint.to_string();
        let messages = Rc::clone(&control.messages);
        let ctx = control.ctx.clone();
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
        let endpoint = endpoint.to_string();
        let active = Rc::clone(&control.active);
        let messages = Rc::clone(&control.messages);
        let ctx = control.ctx.clone();
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

    Ok(WebSocketConnection {
        socket,
        _on_open: on_open,
        _on_message: on_message,
        _on_error: on_error,
        _on_close: on_close,
    })
}

fn begin_cloud_connect(control: IngressControl, endpoint: String, generation: u64) {
    {
        let mut connections = control.connections.borrow_mut();
        let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(&endpoint) else {
            return;
        };
        if connection.generation != generation || connection.connecting {
            return;
        }
        connection.connecting = true;
    }

    spawn_local(async move {
        match request_stream_ticket(&endpoint).await {
            Ok(ticket) => {
                if let Err(error) = open_cloud_socket(&control, &endpoint, generation, &ticket) {
                    set_cloud_connecting(&control, &endpoint, generation, false);
                    schedule_cloud_retry(control, endpoint, generation, error);
                }
            }
            Err(error) => {
                set_cloud_connecting(&control, &endpoint, generation, false);
                schedule_cloud_retry(control, endpoint, generation, error);
            }
        }
    });
}

fn open_cloud_socket(
    control: &IngressControl,
    endpoint: &str,
    generation: u64,
    ticket: &AcquiredStreamTicket,
) -> Result<(), String> {
    let Some(after) = prepare_cloud_tenant(control, endpoint, generation, ticket.tenant_id) else {
        return Ok(());
    };
    let stream_url = stream_endpoint(endpoint, &ticket.value, after)?;
    let socket = WebSocket::new(&stream_url)
        .map_err(|_| "failed to open the authenticated cloud stream".to_string())?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let on_open = {
        let control = control.clone();
        let endpoint = endpoint.to_string();
        Closure::wrap(Box::new(move |_event: Event| {
            let mut connections = control.connections.borrow_mut();
            let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(&endpoint) else {
                return;
            };
            if connection.generation == generation {
                connection.connecting = false;
                connection.reconnect_attempt = 0;
            }
        }) as Box<dyn FnMut(Event)>)
    };

    let on_message = {
        let control = control.clone();
        let endpoint = endpoint.to_string();
        let tenant_id = ticket.tenant_id;
        Closure::wrap(Box::new(
            move |event: MessageEvent| match decode_cloud_message(&event) {
                Ok(record) => {
                    queue_cloud_record(&control, &endpoint, generation, tenant_id, record)
                }
                Err(error) => control.report_error(Some(endpoint.clone()), error),
            },
        ) as Box<dyn FnMut(MessageEvent)>)
    };

    let on_error = {
        let control = control.clone();
        let endpoint = endpoint.to_string();
        Closure::wrap(Box::new(move |_event: Event| {
            control.report_error(
                Some(endpoint.clone()),
                "cloud signal stream WebSocket error".to_string(),
            );
        }) as Box<dyn FnMut(Event)>)
    };

    let on_close = {
        let control = control.clone();
        let endpoint = endpoint.to_string();
        Closure::wrap(Box::new(move |event: CloseEvent| {
            set_cloud_connecting(&control, &endpoint, generation, false);
            let detail = if event.was_clean() {
                "cloud signal stream closed; reconnecting".to_string()
            } else {
                format!(
                    "cloud signal stream closed with code {}; reconnecting",
                    event.code()
                )
            };
            schedule_cloud_retry(control.clone(), endpoint.clone(), generation, detail);
        }) as Box<dyn FnMut(CloseEvent)>)
    };

    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let connection = WebSocketConnection {
        socket,
        _on_open: on_open,
        _on_message: on_message,
        _on_error: on_error,
        _on_close: on_close,
    };
    let mut connections = control.connections.borrow_mut();
    let Some(BrowserConnection::Cloud(cloud)) = connections.get_mut(endpoint) else {
        connection.detach_handlers();
        let _ = connection.close();
        return Ok(());
    };
    if cloud.generation != generation {
        connection.detach_handlers();
        let _ = connection.close();
        return Ok(());
    }
    if let Some(previous) = cloud.socket.take() {
        previous.detach_handlers();
        let _ = previous.close();
    }
    cloud.socket = Some(connection);
    Ok(())
}

fn prepare_cloud_tenant(
    control: &IngressControl,
    endpoint: &str,
    generation: u64,
    tenant_id: TenantId,
) -> Option<u64> {
    let current = {
        let connections = control.connections.borrow();
        let BrowserConnection::Cloud(connection) = connections.get(endpoint)? else {
            return None;
        };
        if connection.generation != generation {
            return None;
        }
        (connection.tenant_id, connection.persisted_cursor)
    };
    if current.0 == Some(tenant_id) {
        return Some(current.1);
    }

    let persisted_cursor = match load_cursor(endpoint, tenant_id) {
        Ok(cursor) => cursor,
        Err(error) => {
            control.report_error(Some(endpoint.to_string()), error);
            0
        }
    };
    {
        let mut connections = control.connections.borrow_mut();
        let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(endpoint) else {
            return None;
        };
        if connection.generation != generation {
            return None;
        }
        connection.tenant_id = Some(tenant_id);
        connection.persisted_cursor = persisted_cursor;
    }
    publish_storage_scope(
        &control.messages,
        &control.ctx,
        BrowserStorageScope::Cloud {
            endpoint: endpoint.to_string(),
            tenant_id,
        },
    );
    Some(persisted_cursor)
}

fn queue_cloud_record(
    control: &IngressControl,
    endpoint: &str,
    generation: u64,
    tenant_id: TenantId,
    record: DecodedJsonSignal,
) {
    let Some(cursor) = record.cursor else {
        control.report_error(
            Some(endpoint.to_string()),
            "cloud stream payload is not a full SignalRecord".to_string(),
        );
        return;
    };
    if record.tenant_id != Some(tenant_id) {
        control.report_error(
            Some(endpoint.to_string()),
            "cloud stream SignalRecord tenant does not match the authenticated stream".to_string(),
        );
        return;
    }
    let should_queue = {
        let connections = control.connections.borrow();
        let Some(BrowserConnection::Cloud(connection)) = connections.get(endpoint) else {
            return;
        };
        connection.generation == generation
            && connection.tenant_id == Some(tenant_id)
            && cursor > connection.persisted_cursor
    };
    if !should_queue {
        return;
    }

    push_message(
        &control.messages,
        &control.ctx,
        IngressMessage::Signal {
            listener: endpoint.to_string(),
            signal: record.signal,
        },
    );

    match persist_cursor(endpoint, tenant_id, cursor) {
        Ok(()) => {
            let mut connections = control.connections.borrow_mut();
            if let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(endpoint)
                && connection.generation == generation
                && connection.tenant_id == Some(tenant_id)
            {
                connection.persisted_cursor = connection.persisted_cursor.max(cursor);
            }
        }
        Err(error) => control.report_error(Some(endpoint.to_string()), error),
    }
}

fn set_cloud_connecting(
    control: &IngressControl,
    endpoint: &str,
    generation: u64,
    connecting: bool,
) {
    let mut connections = control.connections.borrow_mut();
    if let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(endpoint)
        && connection.generation == generation
    {
        connection.connecting = connecting;
    }
}

fn schedule_cloud_retry(
    control: IngressControl,
    endpoint: String,
    generation: u64,
    reason: String,
) {
    let attempt = {
        let mut connections = control.connections.borrow_mut();
        let Some(BrowserConnection::Cloud(connection)) = connections.get_mut(&endpoint) else {
            return;
        };
        if connection.generation != generation || connection.connecting {
            return;
        }
        if connection.reconnect_attempt >= MAX_RECONNECT_ATTEMPTS {
            control.report_error(
                Some(endpoint),
                "cloud reconnect limit reached; disconnect and connect again to retry".to_string(),
            );
            return;
        }
        connection.reconnect_attempt += 1;
        connection.reconnect_attempt
    };

    let delay_ms =
        (1_000_u32.saturating_mul(1_u32 << u32::from(attempt - 1))).min(MAX_RECONNECT_DELAY_MS);
    control.report_error(
        Some(endpoint.clone()),
        format!(
            "{reason} (retry {attempt}/{MAX_RECONNECT_ATTEMPTS} in {}s)",
            delay_ms / 1_000
        ),
    );

    spawn_local(async move {
        TimeoutFuture::new(delay_ms).await;
        let still_current = control.connections.borrow().get(&endpoint).is_some_and(
            |connection| {
                matches!(connection, BrowserConnection::Cloud(connection) if connection.generation == generation)
            },
        );
        if still_current {
            begin_cloud_connect(control, endpoint, generation);
        }
    });
}

#[derive(Deserialize)]
struct StreamTicketResponse {
    ticket: String,
}

struct AcquiredStreamTicket {
    value: String,
    tenant_id: TenantId,
}

async fn request_stream_ticket(cloud_api: &str) -> Result<AcquiredStreamTicket, String> {
    // The host OIDC/PKCE shell owns refresh. Read the latest token for every
    // attempt and never retain it in the Rust application state.
    let token = access_token()?;
    let endpoint = ticket_endpoint(cloud_api)?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_cache(RequestCache::NoStore);
    init.set_mode(RequestMode::Cors);
    let headers =
        Headers::new().map_err(|_| "failed to create ticket request headers".to_string())?;
    headers
        .set("Authorization", &format!("Bearer {token}"))
        .map_err(|_| "failed to set ticket authorization".to_string())?;
    headers
        .set("Accept", "application/json")
        .map_err(|_| "failed to set ticket response type".to_string())?;
    headers
        .set("Cache-Control", "no-store")
        .map_err(|_| "failed to disable ticket request caching".to_string())?;
    headers
        .set("Pragma", "no-cache")
        .map_err(|_| "failed to disable ticket request caching".to_string())?;
    init.set_headers(&headers);
    let request = Request::new_with_str_and_init(&endpoint, &init)
        .map_err(|_| "failed to create stream-ticket request".to_string())?;
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_string())?;
    let response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| "stream-ticket request failed".to_string())?
        .dyn_into::<Response>()
        .map_err(|_| "stream-ticket response has an invalid type".to_string())?;
    if !response.ok() {
        return Err(format!(
            "stream-ticket request was rejected with HTTP {}",
            response.status()
        ));
    }
    let body = JsFuture::from(
        response
            .text()
            .map_err(|_| "failed to read stream-ticket response".to_string())?,
    )
    .await
    .map_err(|_| "failed to read stream-ticket response".to_string())?
    .as_string()
    .ok_or_else(|| "stream-ticket response is not text".to_string())?;
    if body.len() > MAX_TICKET_RESPONSE_BYTES {
        return Err("stream-ticket response is too large".to_string());
    }
    let response: StreamTicketResponse = serde_json::from_str(&body)
        .map_err(|_| "stream-ticket response is invalid JSON".to_string())?;
    let ticket = response.ticket.trim();
    if ticket.is_empty() || ticket.len() > MAX_TICKET_BYTES || ticket.chars().any(char::is_control)
    {
        return Err("stream-ticket response contains an invalid ticket".to_string());
    }
    let tenant_id = tenant_from_stream_ticket(ticket)?;
    Ok(AcquiredStreamTicket {
        value: ticket.to_string(),
        tenant_id,
    })
}

fn access_token() -> Result<String, String> {
    let storage = web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_string())?
        .session_storage()
        .map_err(js_error)?
        .ok_or_else(|| "browser sessionStorage is unavailable".to_string())?;
    let token = storage
        .get_item(ACCESS_TOKEN_SESSION_KEY)
        .map_err(js_error)?
        .unwrap_or_default();
    let token = token.trim();
    if token.is_empty() {
        return Err(format!(
            "Keycloak access token is missing from sessionStorage key {ACCESS_TOKEN_SESSION_KEY}"
        ));
    }
    if token.len() > MAX_ACCESS_TOKEN_BYTES || token.chars().any(char::is_control) {
        return Err("Keycloak access token in sessionStorage is invalid".to_string());
    }
    Ok(token.to_string())
}

fn load_cursor(cloud_api: &str, tenant_id: TenantId) -> Result<u64, String> {
    let storage = local_storage()?;
    let Some(value) = storage
        .get_item(&cursor_storage_key(cloud_api, tenant_id))
        .map_err(js_error)?
    else {
        return Ok(0);
    };
    value
        .parse::<u64>()
        .map_err(|_| "stored cloud stream cursor is invalid; starting from cursor zero".to_string())
}

fn persist_cursor(cloud_api: &str, tenant_id: TenantId, cursor: u64) -> Result<(), String> {
    local_storage()?
        .set_item(
            &cursor_storage_key(cloud_api, tenant_id),
            &cursor.to_string(),
        )
        .map_err(|_| "failed to persist the cloud stream cursor in localStorage".to_string())
}

fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or_else(|| "browser window is unavailable".to_string())?
        .local_storage()
        .map_err(js_error)?
        .ok_or_else(|| "browser localStorage is unavailable".to_string())
}

fn configured_endpoints() -> Vec<String> {
    let Some(window) = web_sys::window() else {
        return Vec::new();
    };
    let Ok(search) = window.location().search() else {
        return Vec::new();
    };
    let Ok(params) = UrlSearchParams::new_with_str(&search) else {
        return Vec::new();
    };
    [params.get("client_api"), params.get("ingress")]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn decode_raw_message(event: &MessageEvent) -> Result<Signal, String> {
    let data = event.data();
    if let Some(text) = data.as_string() {
        // A raw bridge may forward either the legacy Signal or a complete
        // cloud record. Its cursor is intentionally ignored: reconnect and
        // replay guarantees only apply to explicit cloud client API mode.
        return decode_json_signal(&text).map(|decoded| decoded.signal);
    }
    if data.is_instance_of::<ArrayBuffer>() {
        let bytes = Uint8Array::new(&data).to_vec();
        return frame::decode_signal_payload(&bytes)
            .map_err(|error| format!("invalid WebSocket signal payload: {error}"));
    }
    Err("unsupported WebSocket message type; expected JSON text or ArrayBuffer".to_string())
}

fn decode_cloud_message(event: &MessageEvent) -> Result<DecodedJsonSignal, String> {
    event
        .data()
        .as_string()
        .ok_or_else(|| "cloud stream payload must be a JSON SignalRecord text message".to_string())
        .and_then(|value| decode_json_signal(&value))
}

fn publish_active_snapshot(
    active: &ActiveConnections,
    messages: &MessageQueue,
    ctx: &eframe::egui::Context,
) {
    let endpoints = active.borrow().iter().cloned().collect();
    push_message(messages, ctx, IngressMessage::Listening(endpoints));
}

fn publish_storage_scope(
    messages: &MessageQueue,
    ctx: &eframe::egui::Context,
    scope: BrowserStorageScope,
) {
    push_message(messages, ctx, IngressMessage::BrowserStorageScope(scope));
}

fn push_message(messages: &MessageQueue, ctx: &eframe::egui::Context, message: IngressMessage) {
    messages.borrow_mut().push_back(message);
    ctx.request_repaint();
}

fn js_error(_error: JsValue) -> String {
    "browser storage operation failed".to_string()
}
