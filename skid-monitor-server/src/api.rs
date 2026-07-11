//! Tenant-scoped client access plane for cloud mode.
//!
//! This process never accepts agent credentials. Every query and stream is
//! authorized with an OIDC user token and constrained to the token's tenant.

use crate::auth::{AuthError, AuthenticatedPrincipal, OidcVerifier};
use crate::config::{ClientServerConfig, TlsMode};
use crate::store::{
    AgentRecord, PgSignalStore, PgStoreError, PgStoreOptions, ProjectionRecord,
    SIGNAL_NOTIFY_CHANNEL, TenantRecord,
};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt, stream};
use serde::{Deserialize, Serialize};
use skid_monitor_core::{AgentId, SignalCursor, SignalRecord, SignalScope, TenantId};
use sqlx::postgres::PgListener;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

const STREAM_PROTOCOL: &str = "skid-monitor-v1";
const STREAM_TICKET_TTL: Duration = Duration::from_secs(30);
const STREAM_SEND_TIMEOUT: Duration = Duration::from_secs(10);
const REST_REPLAY_CHUNK_BYTES: usize = 64 * 1024;

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Clone)]
struct AppState {
    store: PgSignalStore,
    auth: OidcVerifier,
    admin_role: Arc<str>,
    stream_batch_size: usize,
    stream_batch_bytes: usize,
    notifications: broadcast::Sender<TenantId>,
    stream_slots: Arc<Semaphore>,
    replay_slots: Arc<Semaphore>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn service_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "store_unavailable",
            message: "signal store is temporarily unavailable".to_string(),
        }
    }

    fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "the operation could not be completed".to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: &self.message,
            }),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct SignalQuery {
    #[serde(default)]
    after: SignalCursor,
    limit: Option<usize>,
}

struct SignalPageStreamState {
    records: std::vec::IntoIter<SignalRecord>,
    next_cursor: SignalCursor,
    prefix_sent: bool,
    first_record: bool,
    suffix_sent: bool,
    pending: Bytes,
    _replay_permit: OwnedSemaphorePermit,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollAgentBody {
    agent_id: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpsertTenantBody {
    slug: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAgentBody {
    enabled: bool,
}

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(default)]
    after: SignalCursor,
    ticket: Option<String>,
}

#[derive(Serialize)]
struct StreamTicketResponse {
    ticket: String,
    expires_in_seconds: u64,
}

#[derive(Deserialize)]
struct SignalNotification {
    tenant_id: TenantId,
}

#[derive(Serialize)]
struct LiveStatus {
    status: &'static str,
}

/// Runs the cloud client-access service.
pub async fn serve(config: ClientServerConfig) -> Result<(), BoxError> {
    let store = PgSignalStore::connect(
        &config.database.url,
        PgStoreOptions {
            max_connections: config.database.max_connections,
            ..PgStoreOptions::default()
        },
    )
    .await?;
    store.verify_ready().await?;
    let auth = OidcVerifier::discover(config.oidc.clone()).await?;
    let (notifications, _) = broadcast::channel(1024);
    tokio::spawn(watch_signal_notifications(
        config.database.url.clone(),
        notifications.clone(),
    ));

    let state = AppState {
        store,
        auth,
        admin_role: Arc::from(config.admin_role),
        stream_batch_size: config.stream_batch_size,
        stream_batch_bytes: config.stream_batch_bytes,
        notifications,
        stream_slots: Arc::new(Semaphore::new(config.max_stream_connections)),
        replay_slots: Arc::new(Semaphore::new(config.replay_concurrency)),
    };
    let app = router(state)
        .layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<axum::body::Body>| {
                tracing::debug_span!(
                    "request",
                    method = %request.method(),
                    path = %request.uri().path(),
                    version = ?request.version(),
                )
            },
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(ConcurrencyLimitLayer::new(config.request_concurrency))
        .layer(RequestBodyLimitLayer::new(config.request_body_limit));

    info!(address = %config.listen_addr, "cloud client access server listening");
    match config.tls {
        TlsMode::Direct { cert, key } => {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            axum_server::bind_rustls(config.listen_addr, tls)
                .serve(app.into_make_service())
                .await?;
        }
        TlsMode::TerminatedUpstream => {
            let listener = TcpListener::bind(config.listen_addr).await?;
            axum::serve(listener, app).await?;
        }
    }
    Ok(())
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/v1/signals", get(signals))
        .route("/v1/projection", get(projection))
        .route("/v1/stream", get(stream))
        .route("/v1/stream-tickets", post(stream_ticket))
        .route("/v1/tenant", put(upsert_tenant))
        .route("/v1/agents", get(agents).post(enroll_agent))
        .route("/v1/agents/{agent_id}", patch(update_agent))
        .with_state(state)
}

async fn live() -> Json<LiveStatus> {
    Json(LiveStatus { status: "ok" })
}

async fn ready(State(state): State<AppState>) -> Result<Json<LiveStatus>, ApiError> {
    state.store.verify_runtime_schema().await.map_err(|error| {
        warn!(%error, "client access readiness check failed");
        ApiError::service_unavailable()
    })?;
    Ok(Json(LiveStatus { status: "ready" }))
}

async fn signals(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SignalQuery>,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers, None).await?;
    let limit = query
        .limit
        .unwrap_or(state.stream_batch_size)
        .clamp(1, state.stream_batch_size);
    let replay_permit = acquire_replay_permit(&state).await?;
    let records = state
        .store
        .load_envelopes_after_bounded(
            SignalScope::tenant(principal.tenant_id),
            query.after,
            limit,
            state.stream_batch_bytes,
        )
        .await
        .map_err(|error| store_error("load signals", error))?;
    let next_cursor = records
        .last()
        .map(|record| record.cursor)
        .unwrap_or(query.after);
    Ok(signal_page_response(records, next_cursor, replay_permit))
}

fn signal_page_response(
    records: Vec<SignalRecord>,
    next_cursor: SignalCursor,
    replay_permit: OwnedSemaphorePermit,
) -> Response {
    let state = SignalPageStreamState {
        records: records.into_iter(),
        next_cursor,
        prefix_sent: false,
        first_record: true,
        suffix_sent: false,
        pending: Bytes::new(),
        _replay_permit: replay_permit,
    };
    let stream = stream::unfold(state, |mut state| async move {
        loop {
            if !state.pending.is_empty() {
                let chunk_len = state.pending.len().min(REST_REPLAY_CHUNK_BYTES);
                let chunk = state.pending.split_to(chunk_len);
                return Some((Ok::<Bytes, std::io::Error>(chunk), state));
            }
            if !state.prefix_sent {
                state.prefix_sent = true;
                state.pending = Bytes::from_static(b"{\"signals\":[");
                continue;
            }
            if let Some(record) = state.records.next() {
                let mut serialized = Vec::new();
                if !state.first_record {
                    serialized.push(b',');
                }
                state.first_record = false;
                if let Err(error) = serde_json::to_writer(&mut serialized, &record) {
                    state.suffix_sent = true;
                    return Some((
                        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                        state,
                    ));
                }
                state.pending = Bytes::from(serialized);
                continue;
            }
            if !state.suffix_sent {
                state.suffix_sent = true;
                let mut serialized = b"],\"next_cursor\":".to_vec();
                if let Err(error) = serde_json::to_writer(&mut serialized, &state.next_cursor) {
                    return Some((
                        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                        state,
                    ));
                }
                serialized.push(b'}');
                state.pending = Bytes::from(serialized);
                continue;
            }
            return None;
        }
    });
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

async fn acquire_replay_permit(state: &AppState) -> Result<OwnedSemaphorePermit, ApiError> {
    state
        .replay_slots
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::service_unavailable())
}

async fn agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AgentRecord>>, ApiError> {
    let principal = authenticate(&state, &headers, None).await?;
    state
        .store
        .list_agents(principal.tenant_id)
        .await
        .map(Json)
        .map_err(|error| store_error("list agents", error))
}

async fn projection(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<ProjectionRecord>>, ApiError> {
    let principal = authenticate(&state, &headers, None).await?;
    state
        .store
        .load_projection(principal.tenant_id)
        .await
        .map(Json)
        .map_err(|error| store_error("load projection", error))
}

async fn upsert_tenant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertTenantBody>,
) -> Result<Json<TenantRecord>, ApiError> {
    let principal = authenticate(&state, &headers, Some(&state.admin_role)).await?;
    if body.slug.trim().is_empty() || body.display_name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "tenant slug and display_name must not be empty",
        ));
    }
    state
        .store
        .upsert_tenant_as(
            principal.tenant_id,
            body.slug,
            body.display_name,
            &principal.subject,
        )
        .await
        .map(Json)
        .map_err(|error| store_error("upsert tenant", error))
}

async fn enroll_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EnrollAgentBody>,
) -> Result<(StatusCode, Json<AgentRecord>), ApiError> {
    let principal = authenticate(&state, &headers, Some(&state.admin_role)).await?;
    let agent_id =
        AgentId::new(body.agent_id).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let record = state
        .store
        .enroll_agent_as(
            principal.tenant_id,
            agent_id,
            body.display_name,
            &principal.subject,
        )
        .await
        .map_err(|error| store_error("enroll agent", error))?;
    Ok((StatusCode::CREATED, Json(record)))
}

async fn update_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(body): Json<UpdateAgentBody>,
) -> Result<Json<AgentRecord>, ApiError> {
    let principal = authenticate(&state, &headers, Some(&state.admin_role)).await?;
    let agent_id =
        AgentId::new(agent_id).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let record = state
        .store
        .set_agent_enabled_as(
            principal.tenant_id,
            &agent_id,
            body.enabled,
            &principal.subject,
        )
        .await
        .map_err(|error| store_error("update agent", error))?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "agent_not_found",
            message: "the agent does not exist in this tenant".to_string(),
        })?;
    Ok(Json(record))
}

async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let stream_permit = state
        .stream_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "stream_capacity_reached",
            message: "stream connection capacity is temporarily exhausted".to_string(),
        })?;
    let (tenant_id, authorized_until_unix) = match query.ticket.as_deref() {
        Some(ticket) => {
            let (tenant_id, ticket_id) = parse_stream_ticket(ticket)?;
            let grant = state
                .store
                .consume_stream_ticket(tenant_id, ticket_id)
                .await
                .map_err(|error| store_error("consume stream ticket", error))?
                .ok_or_else(invalid_stream_ticket)?;
            (tenant_id, grant.authorized_until_unix)
        }
        None => {
            let principal = authenticate(&state, &headers, None).await?;
            (principal.tenant_id, principal.authorized_until_unix)
        }
    };
    if authorized_until_unix <= unix_now() {
        return Err(auth_error(AuthError::InvalidToken(
            "access token expired".to_string(),
        )));
    }
    Ok(websocket
        .protocols([STREAM_PROTOCOL])
        .on_upgrade(move |socket| {
            stream_signals(
                socket,
                state,
                tenant_id,
                query.after,
                authorized_until_unix,
                stream_permit,
            )
        }))
}

async fn stream_ticket(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let principal = authenticate(&state, &headers, None).await?;
    let ticket_id = state
        .store
        .create_stream_ticket(
            principal.tenant_id,
            &principal.subject,
            principal.authorized_until_unix,
            STREAM_TICKET_TTL,
        )
        .await
        .map_err(|error| store_error("create stream ticket", error))?;
    let expires_in_seconds = STREAM_TICKET_TTL
        .as_secs()
        .min(principal.authorized_until_unix.saturating_sub(unix_now()));
    let mut response = (
        StatusCode::CREATED,
        Json(StreamTicketResponse {
            ticket: format!("{}.{}", principal.tenant_id, ticket_id),
            expires_in_seconds,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

async fn stream_signals(
    socket: WebSocket,
    state: AppState,
    tenant_id: TenantId,
    mut cursor: SignalCursor,
    authorized_until_unix: u64,
    _stream_permit: OwnedSemaphorePermit,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut notifications = state.notifications.subscribe();
    let mut fallback_poll = tokio::time::interval(Duration::from_secs(2));
    fallback_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Consume the immediate first tick. Later ticks close the LISTEN setup and
    // reconnect race without turning the stream into a busy poll loop.
    fallback_poll.tick().await;
    let authorization_expiry = tokio::time::sleep(Duration::from_secs(
        authorized_until_unix.saturating_sub(unix_now()),
    ));
    tokio::pin!(authorization_expiry);

    loop {
        if unix_now() >= authorized_until_unix {
            let _ = tokio::time::timeout(Duration::from_secs(1), sender.send(Message::Close(None)))
                .await;
            return;
        }
        let replay_slots = Arc::clone(&state.replay_slots);
        let replay_permit = tokio::select! {
            permit = replay_slots.acquire_owned() => {
                match permit {
                    Ok(permit) => permit,
                    Err(_) => {
                        warn!(%tenant_id, "cloud signal replay capacity closed");
                        return;
                    }
                }
            }
            _ = &mut authorization_expiry => {
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    sender.send(Message::Close(None)),
                ).await;
                return;
            }
        };
        let records = match state
            .store
            .load_envelopes_after_bounded(
                SignalScope::tenant(tenant_id),
                cursor,
                state.stream_batch_size,
                state.stream_batch_bytes,
            )
            .await
        {
            Ok(records) => records,
            Err(error) => {
                warn!(%error, %tenant_id, "cloud signal stream query failed");
                return;
            }
        };
        let had_records = !records.is_empty();
        for record in records {
            if unix_now() >= authorized_until_unix {
                let _ =
                    tokio::time::timeout(Duration::from_secs(1), sender.send(Message::Close(None)))
                        .await;
                return;
            }
            let message = match serde_json::to_string(&record) {
                Ok(payload) => Message::Text(payload.into()),
                Err(error) => {
                    warn!(%error, "failed to serialize cloud signal stream item");
                    return;
                }
            };
            cursor = record.cursor;
            let send_timeout = tokio::time::sleep(STREAM_SEND_TIMEOUT);
            tokio::pin!(send_timeout);
            tokio::select! {
                result = sender.send(message) => {
                    if result.is_err() {
                        return;
                    }
                }
                _ = &mut authorization_expiry => return,
                _ = &mut send_timeout => {
                    warn!(%tenant_id, "closing slow cloud signal stream");
                    return;
                }
            }
        }
        drop(replay_permit);
        if had_records {
            continue;
        }

        loop {
            tokio::select! {
                notification = notifications.recv() => {
                    match notification {
                        Ok(changed_tenant) if changed_tenant != tenant_id => continue,
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => break,
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
                _ = fallback_poll.tick() => break,
                _ = &mut authorization_expiry => {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        sender.send(Message::Close(None)),
                    ).await;
                    return;
                }
                message = receiver.next() => {
                    match message {
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                        Some(Ok(Message::Ping(payload))) => {
                            if !matches!(
                                tokio::time::timeout(
                                    STREAM_SEND_TIMEOUT,
                                    sender.send(Message::Pong(payload)),
                                ).await,
                                Ok(Ok(()))
                            ) {
                                return;
                            }
                        }
                        Some(Ok(_)) => {}
                    }
                }
            }
        }
    }
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    required_role: Option<&str>,
) -> Result<AuthenticatedPrincipal, ApiError> {
    let authorization = authorization(headers)?;
    let result = match required_role {
        Some(role) => {
            state
                .auth
                .verify_bearer_for_role(&authorization, role)
                .await
        }
        None => state.auth.verify_bearer(&authorization).await,
    };
    result.map_err(auth_error)
}

fn authorization(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| auth_error(AuthError::MissingBearer))?
        .to_str()
        .map(str::to_string)
        .map_err(|_| auth_error(AuthError::InvalidBearer))
}

fn parse_stream_ticket(value: &str) -> Result<(TenantId, Uuid), ApiError> {
    let (tenant, ticket) = value.split_once('.').ok_or_else(invalid_stream_ticket)?;
    let tenant_id = tenant.parse().map_err(|_| invalid_stream_ticket())?;
    let ticket_id = Uuid::parse_str(ticket).map_err(|_| invalid_stream_ticket())?;
    Ok((tenant_id, ticket_id))
}

fn invalid_stream_ticket() -> ApiError {
    ApiError {
        status: StatusCode::UNAUTHORIZED,
        code: "invalid_stream_ticket",
        message: "the one-time stream ticket is invalid or expired".to_string(),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn auth_error(error: AuthError) -> ApiError {
    if matches!(&error, AuthError::Discovery(_) | AuthError::KeySet(_)) {
        warn!(%error, "OIDC verification backend is unavailable");
    }
    let (status, code) = match error {
        AuthError::MissingRole(_) => (StatusCode::FORBIDDEN, "forbidden"),
        AuthError::Discovery(_) | AuthError::KeySet(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_unavailable",
        ),
        _ => (StatusCode::UNAUTHORIZED, "unauthorized"),
    };
    ApiError {
        status,
        code,
        message: if status == StatusCode::FORBIDDEN {
            "the access token does not grant this operation".to_string()
        } else if status == StatusCode::SERVICE_UNAVAILABLE {
            "the authentication service is temporarily unavailable".to_string()
        } else {
            "a valid OIDC access token is required".to_string()
        },
    }
}

fn store_error(operation: &'static str, error: PgStoreError) -> ApiError {
    warn!(%error, operation, "cloud signal store operation failed");
    match error {
        PgStoreError::Database(_)
        | PgStoreError::Migration(_)
        | PgStoreError::RuntimeSchemaNotReady { .. } => ApiError::service_unavailable(),
        PgStoreError::TenantDisabled { .. } => ApiError {
            status: StatusCode::FORBIDDEN,
            code: "tenant_disabled",
            message: "the tenant is disabled".to_string(),
        },
        PgStoreError::AgentLimitReached { max_agents, .. } => ApiError {
            status: StatusCode::CONFLICT,
            code: "agent_limit_reached",
            message: format!("the tenant cannot enroll more than {max_agents} agents"),
        },
        PgStoreError::StreamTicketAuthorizationExpired => {
            auth_error(AuthError::InvalidToken("access token expired".to_string()))
        }
        _ => ApiError::internal(),
    }
}

async fn watch_signal_notifications(database_url: String, sender: broadcast::Sender<TenantId>) {
    loop {
        match PgListener::connect(&database_url).await {
            Ok(mut listener) => {
                if let Err(error) = listener.listen(SIGNAL_NOTIFY_CHANNEL).await {
                    warn!(%error, "failed to subscribe to PostgreSQL signal notifications");
                } else {
                    loop {
                        match listener.recv().await {
                            Ok(notification) => {
                                match serde_json::from_str::<SignalNotification>(
                                    notification.payload(),
                                ) {
                                    Ok(notification) => {
                                        let _ = sender.send(notification.tenant_id);
                                    }
                                    Err(error) => warn!(
                                        %error,
                                        "ignored malformed PostgreSQL signal notification"
                                    ),
                                }
                            }
                            Err(error) => {
                                warn!(%error, "PostgreSQL signal notification connection lost");
                                break;
                            }
                        }
                    }
                }
            }
            Err(error) => warn!(%error, "failed to connect PostgreSQL signal listener"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn stream_ticket_contains_tenant_and_opaque_id() {
        let tenant = TenantId::from_uuid(Uuid::new_v4());
        let ticket = Uuid::new_v4();
        assert_eq!(
            parse_stream_ticket(&format!("{tenant}.{ticket}")).unwrap(),
            (tenant, ticket)
        );
        assert!(parse_stream_ticket("not-a-ticket").is_err());
    }

    #[test]
    fn authorization_header_takes_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer direct"),
        );
        assert_eq!(authorization(&headers).unwrap(), "Bearer direct");
    }

    #[tokio::test]
    async fn signal_page_stream_preserves_json_and_holds_replay_permit_to_eof() {
        let slots = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&slots).acquire_owned().await.unwrap();
        let response = signal_page_response(Vec::new(), SignalCursor(9), permit);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(slots.available_permits(), 0);

        let mut body = response.into_body().into_data_stream();
        let first = body.next().await.unwrap().unwrap();
        assert_eq!(first, Bytes::from_static(b"{\"signals\":["));
        assert_eq!(slots.available_permits(), 0);

        let mut bytes = first.to_vec();
        while let Some(chunk) = body.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(slots.available_permits(), 1);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"signals": [], "next_cursor": 9})
        );
    }

    #[test]
    fn agent_cardinality_limit_has_a_stable_client_error() {
        let error = store_error(
            "enroll agent",
            PgStoreError::AgentLimitReached {
                tenant_id: TenantId::from_uuid(Uuid::new_v4()),
                max_agents: 1_000,
            },
        );
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code, "agent_limit_reached");
        assert!(error.message.contains("1000"));
    }
}
