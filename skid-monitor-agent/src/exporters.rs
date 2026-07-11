//! Concrete exporter implementations.

use crate::config::{AgentConfig, ExporterConfig, OtlpAuthConfig};
use crate::transport;
use serde::Deserialize;
use skid_protocol::otlp::tonic::collector::logs::v1::logs_service_client::LogsServiceClient;
use skid_protocol::otlp::tonic::collector::metrics::v1::metrics_service_client::MetricsServiceClient;
use skid_protocol::otlp::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
use skid_protocol::protocol::Signal;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tonic::metadata::AsciiMetadataValue;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Code, Request, Status};
use tracing::info;

const OTLP_MAX_ATTEMPTS: usize = 3;
const OTLP_RETRY_BASE_DELAY: Duration = Duration::from_millis(100);
const MAX_OAUTH_TOKEN_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_OAUTH_TOKEN_LIFETIME_SECS: u64 = 24 * 60 * 60;
const MAX_SEQUENCE_STATE_BYTES: u64 = 64;
const SEQUENCE_TEMP_ATTEMPTS: usize = 32;
static SEQUENCE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct SignalExporters {
    inner: Arc<BTreeMap<String, RuntimeExporter>>,
}

#[derive(Debug, Clone)]
enum RuntimeExporter {
    SkidClient { addr: Option<String> },
    Logging { include_json: bool },
    Otlp(Arc<OtlpExporter>),
}

impl SignalExporters {
    pub fn from_config(config: &AgentConfig) -> Result<Self, String> {
        let mut inner = BTreeMap::new();
        for (name, exporter) in &config.exporters {
            let runtime = match exporter {
                ExporterConfig::SkidClient { addr } => {
                    RuntimeExporter::SkidClient { addr: addr.clone() }
                }
                ExporterConfig::Logging { include_json } => RuntimeExporter::Logging {
                    include_json: *include_json,
                },
                ExporterConfig::Otlp { endpoint, auth } => {
                    let exporter = OtlpExporter::new(endpoint, auth.as_ref())
                        .map_err(|error| format!("initialize OTLP exporter {name:?}: {error}"))?;
                    RuntimeExporter::Otlp(Arc::new(exporter))
                }
            };
            inner.insert(name.clone(), runtime);
        }

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    pub async fn export(&self, name: &str, signal: &Signal) -> Result<(), RequiredExporterFailure> {
        let exporter = self.inner.get(name).ok_or_else(|| {
            RequiredExporterFailure::new(name, "pipeline references a missing exporter")
        })?;
        export_one(name, exporter, signal)
            .await
            .map_err(|error| RequiredExporterFailure::new(name, error))
    }
}

/// Failure from one exporter listed by a signal pipeline.
///
/// Every listed exporter is required until delivery policy becomes explicitly
/// configurable, so callers must propagate or otherwise handle this error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequiredExporterFailure {
    exporter: String,
    message: String,
}

impl RequiredExporterFailure {
    fn new(exporter: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            exporter: exporter.into(),
            message: message.into(),
        }
    }

    pub(crate) fn exporter(&self) -> &str {
        &self.exporter
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for RequiredExporterFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "required exporter {:?} failed: {}",
            self.exporter, self.message
        )
    }
}

impl std::error::Error for RequiredExporterFailure {}

async fn export_one(name: &str, exporter: &RuntimeExporter, signal: &Signal) -> Result<(), String> {
    match exporter {
        RuntimeExporter::SkidClient { addr } => {
            let resolved_addr = resolve_client_addr(addr.as_ref());
            transport::send_to_client(signal, resolved_addr.as_deref())
        }
        RuntimeExporter::Logging { include_json } => {
            if *include_json {
                let json = serde_json::to_string(signal)
                    .map_err(|err| format!("serialize signal for logging exporter: {err}"))?;
                info!(
                    exporter = name,
                    signal = signal.kind(),
                    count = signal.item_count(),
                    %json,
                    "signal exported to log"
                );
            } else {
                info!(
                    exporter = name,
                    signal = signal.kind(),
                    count = signal.item_count(),
                    "signal exported to log"
                );
            }
            Ok(())
        }
        RuntimeExporter::Otlp(exporter) => exporter.export(signal).await,
    }
}

#[derive(Debug)]
struct OtlpExporter {
    channel: Channel,
    auth: Option<Arc<OAuthTokenProvider>>,
    sequence: Sequence,
}

impl OtlpExporter {
    fn new(endpoint: &str, auth: Option<&OtlpAuthConfig>) -> Result<Self, String> {
        let endpoint = normalize_endpoint(endpoint);
        let use_tls = endpoint.starts_with("https://");
        if auth.is_some() {
            let url = reqwest::Url::parse(&endpoint)
                .map_err(|_| "authenticated OTLP endpoint is invalid".to_string())?;
            if !use_tls
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(
                    "authenticated OTLP endpoint must use HTTPS without credentials or a fragment"
                        .to_string(),
                );
            }
        }
        let mut endpoint = Endpoint::from_shared(endpoint)
            .map_err(|err| format!("invalid OTLP endpoint: {err}"))?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));
        if use_tls {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_webpki_roots())
                .map_err(|err| format!("configure OTLP TLS: {err}"))?;
        }

        let sequence = match auth {
            Some(config) => Sequence::durable(&config.sequence_state_path)?,
            None => Sequence::ephemeral(initial_sequence()),
        };
        let auth = auth.map(OAuthTokenProvider::new).transpose()?.map(Arc::new);

        Ok(Self {
            channel: endpoint.connect_lazy(),
            auth,
            sequence,
        })
    }

    async fn export(&self, signal: &Signal) -> Result<(), String> {
        // Durable sequence allocation performs fsync + atomic rename and must
        // complete before the first network attempt. It is dispatched through
        // spawn_blocking so filesystem latency never blocks the async reactor.
        let sequence = self.sequence.next().await?;
        let mut refreshed_auth = false;

        for attempt in 1..=OTLP_MAX_ATTEMPTS {
            match self.export_attempt(signal, sequence).await {
                Ok(()) => return Ok(()),
                Err(ExportAttemptError::Prepare(error)) => return Err(error),
                Err(ExportAttemptError::Grpc(status)) => {
                    let decision =
                        retry_decision(status.code(), self.auth.is_some(), refreshed_auth);
                    if attempt == OTLP_MAX_ATTEMPTS || decision == RetryDecision::Stop {
                        return Err(format!(
                            "export OTLP {} after {attempt} attempt(s): {status}",
                            signal.kind()
                        ));
                    }

                    if decision == RetryDecision::RefreshAuth {
                        // `retry_decision` only returns RefreshAuth when auth is
                        // configured and this logical export has not refreshed yet.
                        if let Some(auth) = &self.auth {
                            auth.invalidate().await;
                        }
                        refreshed_auth = true;
                    }
                    tokio::time::sleep(retry_backoff(attempt)).await;
                }
            }
        }

        unreachable!("OTLP retry loop either succeeds or returns its final error")
    }

    async fn export_attempt(
        &self,
        signal: &Signal,
        sequence: u64,
    ) -> Result<(), ExportAttemptError> {
        match signal {
            Signal::Metrics(payload) => {
                let request = self
                    .request(payload.clone(), sequence)
                    .await
                    .map_err(ExportAttemptError::Prepare)?;
                MetricsServiceClient::new(self.channel.clone())
                    .export(request)
                    .await
                    .map_err(ExportAttemptError::Grpc)?;
            }
            Signal::Traces(payload) => {
                let request = self
                    .request(payload.clone(), sequence)
                    .await
                    .map_err(ExportAttemptError::Prepare)?;
                TraceServiceClient::new(self.channel.clone())
                    .export(request)
                    .await
                    .map_err(ExportAttemptError::Grpc)?;
            }
            Signal::Logs(payload) => {
                let request = self
                    .request(payload.clone(), sequence)
                    .await
                    .map_err(ExportAttemptError::Prepare)?;
                LogsServiceClient::new(self.channel.clone())
                    .export(request)
                    .await
                    .map_err(ExportAttemptError::Grpc)?;
            }
        }
        Ok(())
    }

    async fn request<T>(&self, payload: T, sequence: u64) -> Result<Request<T>, String> {
        let token = match &self.auth {
            Some(auth) => Some(auth.access_token().await?),
            None => None,
        };
        let mut request = Request::new(payload);
        request
            .metadata_mut()
            .insert("x-skid-sequence", AsciiMetadataValue::from(sequence));

        if let Some(token) = token {
            let authorization = format!("Bearer {token}");
            let mut value = AsciiMetadataValue::try_from(authorization.as_str())
                .map_err(|_| "OAuth access token cannot be encoded as gRPC metadata".to_string())?;
            value.set_sensitive(true);
            request.metadata_mut().insert("authorization", value);
        }
        Ok(request)
    }
}

enum ExportAttemptError {
    Prepare(String),
    Grpc(Status),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryDecision {
    Retry,
    RefreshAuth,
    Stop,
}

fn retry_decision(code: Code, has_auth: bool, refreshed_auth: bool) -> RetryDecision {
    if matches!(
        code,
        Code::Unavailable
            | Code::DeadlineExceeded
            | Code::Unknown
            | Code::ResourceExhausted
            | Code::Aborted
    ) {
        RetryDecision::Retry
    } else if code == Code::Unauthenticated && has_auth && !refreshed_auth {
        RetryDecision::RefreshAuth
    } else {
        RetryDecision::Stop
    }
}

fn retry_backoff(completed_attempts: usize) -> Duration {
    let multiplier = 1_u32 << completed_attempts.saturating_sub(1).min(2);
    OTLP_RETRY_BASE_DELAY * multiplier
}

struct OAuthTokenProvider {
    config: OtlpAuthConfig,
    client: reqwest::Client,
    cache: Mutex<Option<CachedToken>>,
}

impl Debug for OAuthTokenProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthTokenProvider")
            .field("token_url", &self.config.token_url)
            .field("client_id", &self.config.client_id)
            .field("client_secret_env", &self.config.client_secret_env)
            .field("scope", &self.config.scope)
            .field("cache", &"<redacted>")
            .finish()
    }
}

struct CachedToken {
    value: String,
    refresh_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default = "default_token_lifetime_secs")]
    expires_in: u64,
}

impl OAuthTokenProvider {
    fn new(config: &OtlpAuthConfig) -> Result<Self, String> {
        // AgentConfig validation normally enforces this; keep the runtime
        // boundary fail-closed for callers that construct a config directly.
        let token_url = reqwest::Url::parse(config.token_url.trim())
            .map_err(|_| "OAuth token_url is invalid".to_string())?;
        if token_url.scheme() != "https"
            || token_url.host_str().is_none()
            || !token_url.username().is_empty()
            || token_url.password().is_some()
            || token_url.fragment().is_some()
        {
            return Err(
                "OAuth token_url must be an HTTPS URL without credentials or a fragment"
                    .to_string(),
            );
        }

        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|err| format!("build OAuth HTTP client: {err}"))?;

        Ok(Self {
            config: config.clone(),
            client,
            cache: Mutex::new(None),
        })
    }

    async fn access_token(&self) -> Result<String, String> {
        // Holding this mutex through refresh intentionally coalesces concurrent
        // refreshes into one Keycloak request.
        let mut cache = self.cache.lock().await;
        if let Some(token) = cache.as_ref()
            && token.refresh_at > Instant::now()
        {
            return Ok(token.value.clone());
        }

        let secret = std::env::var(&self.config.client_secret_env).map_err(|_| {
            format!(
                "OAuth client secret environment variable {:?} is not set",
                self.config.client_secret_env
            )
        })?;
        if secret.is_empty() {
            return Err(format!(
                "OAuth client secret environment variable {:?} is empty",
                self.config.client_secret_env
            ));
        }

        let mut form = vec![("grant_type", "client_credentials")];
        if let Some(scope) = self.config.scope.as_deref() {
            form.push(("scope", scope));
        }
        let response = self
            .client
            .post(self.config.token_url.trim())
            .basic_auth(&self.config.client_id, Some(secret))
            .form(&form)
            .send()
            .await
            .map_err(|err| format!("request OAuth access token: {err}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "OAuth token endpoint returned HTTP {}",
                response.status()
            ));
        }
        let body = read_bounded_token_response(response).await?;
        let response: TokenResponse = serde_json::from_slice(&body)
            .map_err(|err| format!("decode OAuth token response: {err}"))?;
        if response.access_token.is_empty() {
            return Err("OAuth token endpoint returned an empty access_token".to_string());
        }
        if response
            .token_type
            .as_deref()
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
        {
            return Err("OAuth token endpoint returned a non-Bearer token".to_string());
        }
        let refresh_at = token_refresh_deadline(Instant::now(), response.expires_in)?;

        let value = response.access_token;
        *cache = Some(CachedToken {
            value: value.clone(),
            refresh_at,
        });
        Ok(value)
    }

    async fn invalidate(&self) {
        self.cache.lock().await.take();
    }
}

async fn read_bounded_token_response(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > MAX_OAUTH_TOKEN_RESPONSE_BYTES as u64) {
        return Err(format!(
            "OAuth token response exceeds {MAX_OAUTH_TOKEN_RESPONSE_BYTES} bytes"
        ));
    }

    let mut body = Vec::with_capacity(content_length.unwrap_or(0) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("read OAuth token response: {err}"))?
    {
        append_bounded_body(&mut body, &chunk, MAX_OAUTH_TOKEN_RESPONSE_BYTES)?;
    }
    Ok(body)
}

fn append_bounded_body(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), String> {
    let next_len = body
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| "OAuth token response size overflow".to_string())?;
    if next_len > limit {
        return Err(format!("OAuth token response exceeds {limit} bytes"));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn token_refresh_deadline(now: Instant, expires_in_secs: u64) -> Result<Instant, String> {
    if expires_in_secs == 0 {
        return Err("OAuth token endpoint returned expires_in=0".to_string());
    }
    if expires_in_secs > MAX_OAUTH_TOKEN_LIFETIME_SECS {
        return Err(format!(
            "OAuth token expires_in exceeds the maximum of {MAX_OAUTH_TOKEN_LIFETIME_SECS} seconds"
        ));
    }
    now.checked_add(refresh_after(expires_in_secs))
        .ok_or_else(|| "OAuth token refresh deadline overflow".to_string())
}

#[derive(Debug)]
enum Sequence {
    /// Backward-compatible sequence for trusted, unauthenticated OTLP.
    Ephemeral(AtomicU64),
    /// Restart-safe sequence required for authenticated cloud ingress.
    Durable(Arc<DurableSequence>),
}

impl Sequence {
    fn ephemeral(start: u64) -> Self {
        Self::Ephemeral(AtomicU64::new(start.max(1)))
    }

    fn durable(path: &Path) -> Result<Self, String> {
        DurableSequence::open(path).map(|sequence| Self::Durable(Arc::new(sequence)))
    }

    async fn next(&self) -> Result<u64, String> {
        match self {
            Self::Ephemeral(sequence) => sequence
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| "OTLP request sequence exhausted".to_string()),
            Self::Durable(sequence) => {
                let sequence = Arc::clone(sequence);
                tokio::task::spawn_blocking(move || sequence.next_blocking())
                    .await
                    .map_err(|error| format!("join OTLP sequence persistence task: {error}"))?
            }
        }
    }
}

/// File-backed allocator whose state is the next sequence value to return.
///
/// The companion lock file deliberately remains on disk after shutdown. Its
/// advisory lock, held by `_lock_file`, is released by the OS when this value
/// is dropped; unlinking it would create an inode race between processes.
#[derive(Debug)]
struct DurableSequence {
    state_path: PathBuf,
    next: StdMutex<u64>,
    _lock_file: File,
}

impl DurableSequence {
    fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_start(path, initial_sequence())
    }

    fn open_with_start(path: &Path, initial: u64) -> Result<Self, String> {
        let state_path = normalize_sequence_state_path(path)?;
        let lock_path = companion_lock_path(&state_path);
        let lock_file = open_sequence_lock(&lock_path)?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "sequence state {:?} is already locked by another exporter/process",
                    state_path
                ));
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "acquire sequence state lock for {:?}: {error}",
                    state_path
                ));
            }
        }

        let next = match fs::symlink_metadata(&state_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(format!(
                        "sequence state {:?} must be a regular, non-symlink file",
                        state_path
                    ));
                }
                read_sequence_state(&state_path, &metadata)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let initial = initial.max(1);
                persist_sequence_state(&state_path, initial)?;
                initial
            }
            Err(error) => {
                return Err(format!("inspect sequence state {:?}: {error}", state_path));
            }
        };

        Ok(Self {
            state_path,
            next: StdMutex::new(next),
            _lock_file: lock_file,
        })
    }

    fn next_blocking(&self) -> Result<u64, String> {
        let mut next = self
            .next
            .lock()
            .map_err(|_| "OTLP sequence state mutex is poisoned".to_string())?;
        let allocated = *next;
        let following = allocated
            .checked_add(1)
            .ok_or_else(|| "OTLP request sequence exhausted".to_string())?;

        // Persist the following value before allowing any network send. A
        // crash may leave a harmless gap, but cannot cause sequence reuse.
        persist_sequence_state(&self.state_path, following)?;
        *next = following;
        Ok(allocated)
    }
}

fn normalize_sequence_state_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err("sequence_state_path must name a file".to_string());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|error| {
        format!(
            "resolve sequence state parent directory {:?}: {error}",
            parent
        )
    })?;
    if !parent.is_dir() {
        return Err(format!(
            "sequence state parent {:?} is not a directory",
            parent
        ));
    }
    Ok(parent.join(path.file_name().expect("validated file name")))
}

fn companion_lock_path(state_path: &Path) -> PathBuf {
    let mut value = state_path.as_os_str().to_os_string();
    value.push(".lock");
    PathBuf::from(value)
}

fn open_sequence_lock(path: &Path) -> Result<File, String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(format!("sequence lock {:?} must not be a symlink", path));
    }

    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    apply_secure_open_options(&mut options);
    let file = options
        .open(path)
        .map_err(|error| format!("open sequence lock {:?}: {error}", path))?;
    if !file
        .metadata()
        .map_err(|error| format!("inspect sequence lock {:?}: {error}", path))?
        .is_file()
    {
        return Err(format!("sequence lock {:?} is not a regular file", path));
    }
    restrict_file_permissions(&file, path)?;
    Ok(file)
}

fn read_sequence_state(path: &Path, metadata: &fs::Metadata) -> Result<u64, String> {
    if metadata.len() == 0 || metadata.len() > MAX_SEQUENCE_STATE_BYTES {
        return Err(format!(
            "sequence state {:?} is corrupt: expected a short decimal u64",
            path
        ));
    }

    let mut options = OpenOptions::new();
    options.read(true);
    apply_secure_open_options(&mut options);
    let mut file = options
        .open(path)
        .map_err(|error| format!("open sequence state {:?}: {error}", path))?;
    restrict_file_permissions(&file, path)?;
    let mut value = String::with_capacity(metadata.len() as usize);
    file.read_to_string(&mut value)
        .map_err(|error| format!("read sequence state {:?}: {error}", path))?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "sequence state {:?} is corrupt: expected a decimal u64",
            path
        ));
    }
    let value = value.parse::<u64>().map_err(|error| {
        format!(
            "sequence state {:?} is corrupt: invalid decimal u64: {error}",
            path
        )
    })?;
    if value == 0 {
        return Err(format!(
            "sequence state {:?} is corrupt: zero is reserved",
            path
        ));
    }
    Ok(value)
}

fn persist_sequence_state(path: &Path, next: u64) -> Result<(), String> {
    let parent = path.parent().expect("normalized path has a parent");
    let file_name = path.file_name().expect("normalized path has a file name");
    let mut created = None;
    for _ in 0..SEQUENCE_TEMP_ATTEMPTS {
        let counter = SEQUENCE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_name = format!(
            ".{}.tmp.{}.{}",
            file_name.to_string_lossy(),
            std::process::id(),
            counter
        );
        let temp_path = parent.join(temp_name);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        apply_secure_open_options(&mut options);
        match options.open(&temp_path) {
            Ok(file) => {
                created = Some((temp_path, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "create sequence state temporary file beside {:?}: {error}",
                    path
                ));
            }
        }
    }
    let (temp_path, mut temp_file) = created.ok_or_else(|| {
        format!(
            "could not create a unique sequence state temporary file beside {:?}",
            path
        )
    })?;

    let persist_result = (|| -> Result<(), String> {
        restrict_file_permissions(&temp_file, &temp_path)?;
        writeln!(temp_file, "{next}")
            .map_err(|error| format!("write sequence state {:?}: {error}", temp_path))?;
        temp_file
            .sync_all()
            .map_err(|error| format!("fsync sequence state {:?}: {error}", temp_path))?;
        drop(temp_file);
        atomic_replace(&temp_path, path).map_err(|error| {
            format!(
                "atomically replace sequence state {:?} from {:?}: {error}",
                path, temp_path
            )
        })?;
        sync_parent_directory(parent)?;
        Ok(())
    })();

    if persist_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    persist_result
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // Same-directory placement keeps this on one volume. REPLACE_EXISTING is
    // required because std::fs::rename does not replace an existing file on
    // Windows; WRITE_THROUGH waits for the move to reach storage.
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn apply_secure_open_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
}

#[cfg(not(unix))]
fn apply_secure_open_options(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn restrict_file_permissions(file: &File, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = file
        .metadata()
        .map_err(|error| format!("inspect permissions for {:?}: {error}", path))?
        .permissions();
    permissions.set_mode(0o600);
    file.set_permissions(permissions)
        .map_err(|error| format!("restrict permissions for {:?}: {error}", path))
}

#[cfg(not(unix))]
fn restrict_file_permissions(_file: &File, _path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), String> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("fsync sequence state directory {:?}: {error}", parent))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), String> {
    Ok(())
}

fn initial_sequence() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(1)
        .max(1)
}

fn default_token_lifetime_secs() -> u64 {
    300
}

fn refresh_after(expires_in_secs: u64) -> Duration {
    // Refresh 20% early for short-lived tokens and up to 60 seconds early
    // for the common Keycloak lifetimes.
    let margin = (expires_in_secs / 5).clamp(1, 60);
    Duration::from_secs(expires_in_secs.saturating_sub(margin))
}

fn resolve_client_addr(configured: Option<&String>) -> Option<String> {
    configured
        .filter(|addr| !addr.trim().is_empty())
        .cloned()
        .or_else(|| env_or_legacy("SKID_MONITOR_CLIENT_ADDR", "MONITOR_CAT_CLIENT_ADDR").ok())
        .filter(|addr| !addr.trim().is_empty())
}

fn normalize_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    }
}

fn env_or_legacy(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(primary).or_else(|_| std::env::var(legacy))
}

trait SignalExt {
    fn kind(&self) -> &'static str;
    fn item_count(&self) -> usize;
}

impl SignalExt for Signal {
    fn kind(&self) -> &'static str {
        match self {
            Signal::Metrics(_) => "metrics",
            Signal::Traces(_) => "traces",
            Signal::Logs(_) => "logs",
        }
    }

    fn item_count(&self) -> usize {
        match self {
            Signal::Metrics(request) => request
                .resource_metrics
                .iter()
                .flat_map(|rm| &rm.scope_metrics)
                .map(|sm| sm.metrics.len())
                .sum(),
            Signal::Traces(request) => request
                .resource_spans
                .iter()
                .flat_map(|rs| &rs.scope_spans)
                .map(|ss| ss.spans.len())
                .sum(),
            Signal::Logs(request) => request
                .resource_logs
                .iter()
                .flat_map(|rl| &rl.scope_logs)
                .map(|sl| sl.log_records.len())
                .sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "skid-monitor-sequence-{label}-{}-{}",
                std::process::id(),
                SEQUENCE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn state_path(&self) -> PathBuf {
            self.0.join("cloud.sequence")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_auth(state_path: PathBuf) -> OtlpAuthConfig {
        OtlpAuthConfig {
            token_url: "https://id.example.test/realms/monitor/protocol/openid-connect/token"
                .to_string(),
            client_id: "agent-one".to_string(),
            client_secret_env: "SKID_TEST_SECRET_NOT_READ".to_string(),
            sequence_state_path: state_path,
            scope: None,
        }
    }

    #[tokio::test]
    async fn sequence_is_strictly_monotonic_and_fails_on_exhaustion() {
        let sequence = Sequence::ephemeral(41);
        assert_eq!(sequence.next().await.unwrap(), 41);
        assert_eq!(sequence.next().await.unwrap(), 42);

        let exhausted = Sequence::ephemeral(u64::MAX);
        assert!(exhausted.next().await.unwrap_err().contains("exhausted"));
    }

    #[test]
    fn durable_sequence_persists_next_value_across_restart() {
        let directory = TestDirectory::new("restart");
        let state_path = directory.state_path();

        let sequence = DurableSequence::open_with_start(&state_path, 41).unwrap();
        assert_eq!(sequence.next_blocking().unwrap(), 41);
        assert_eq!(sequence.next_blocking().unwrap(), 42);
        assert_eq!(fs::read_to_string(&state_path).unwrap(), "43\n");
        drop(sequence);

        let restarted = DurableSequence::open_with_start(&state_path, 1).unwrap();
        assert_eq!(restarted.next_blocking().unwrap(), 43);
        assert_eq!(fs::read_to_string(&state_path).unwrap(), "44\n");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn durable_sequence_rejects_corrupt_state() {
        let directory = TestDirectory::new("corrupt");
        let state_path = directory.state_path();
        fs::write(&state_path, b"not-a-sequence\n").unwrap();

        let error = DurableSequence::open_with_start(&state_path, 1).unwrap_err();
        assert!(error.contains("corrupt"), "{error}");
    }

    #[test]
    fn durable_sequence_rejects_concurrent_process_lock() {
        let directory = TestDirectory::new("lock");
        let state_path = directory.state_path();
        let first = DurableSequence::open_with_start(&state_path, 10).unwrap();

        let error = DurableSequence::open_with_start(&state_path, 10).unwrap_err();
        assert!(error.contains("already locked"), "{error}");
        drop(first);

        DurableSequence::open_with_start(&state_path, 10).unwrap();
    }

    #[tokio::test]
    async fn locked_cloud_sequence_fails_exporter_startup_construction() {
        let directory = TestDirectory::new("startup-lock");
        let state_path = directory.state_path();
        let mut config = AgentConfig::default();
        config.exporters.insert(
            "cloud".to_string(),
            ExporterConfig::Otlp {
                endpoint: "https://ingress.example.test:4317".to_string(),
                auth: Some(test_auth(state_path)),
            },
        );

        let first = SignalExporters::from_config(&config).unwrap();
        let error = SignalExporters::from_config(&config).unwrap_err();
        assert!(
            error.contains("initialize OTLP exporter \"cloud\""),
            "{error}"
        );
        assert!(error.contains("already locked"), "{error}");
        drop(first);
    }

    #[test]
    fn token_refresh_is_scheduled_before_expiry() {
        assert_eq!(refresh_after(100), Duration::from_secs(80));
        assert_eq!(refresh_after(3_600), Duration::from_secs(3_540));
        assert_eq!(refresh_after(1), Duration::ZERO);
    }

    #[test]
    fn token_expiry_must_be_nonzero_and_at_most_one_day() {
        let now = Instant::now();
        assert!(token_refresh_deadline(now, 0).is_err());
        assert!(token_refresh_deadline(now, MAX_OAUTH_TOKEN_LIFETIME_SECS).is_ok());
        assert!(token_refresh_deadline(now, MAX_OAUTH_TOKEN_LIFETIME_SECS + 1).is_err());
    }

    #[test]
    fn chunked_token_body_is_rejected_before_crossing_its_limit() {
        let mut body = Vec::new();
        append_bounded_body(&mut body, b"ab", 4).unwrap();
        append_bounded_body(&mut body, b"cd", 4).unwrap();
        assert_eq!(body, b"abcd");

        let error = append_bounded_body(&mut body, b"e", 4).unwrap_err();
        assert!(error.contains("exceeds 4 bytes"));
        assert_eq!(body, b"abcd");
    }

    #[tokio::test]
    async fn caller_allocates_a_new_sequence_for_each_logical_export() {
        let exporter = OtlpExporter::new("http://127.0.0.1:4317", None).unwrap();
        let first_sequence = exporter.sequence.next().await.unwrap();
        let second_sequence = exporter.sequence.next().await.unwrap();
        let first = exporter.request((), first_sequence).await.unwrap();
        let second = exporter.request((), second_sequence).await.unwrap();
        let first = first
            .metadata()
            .get("x-skid-sequence")
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let second = second
            .metadata()
            .get("x-skid-sequence")
            .unwrap()
            .to_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();

        assert_eq!(second, first + 1);
        assert!(
            exporter
                .request((), exporter.sequence.next().await.unwrap())
                .await
                .unwrap()
                .metadata()
                .get("authorization")
                .is_none()
        );
    }

    #[tokio::test]
    async fn retry_attempts_reuse_one_durable_allocation() {
        let directory = TestDirectory::new("retry");
        let state_path = directory.state_path();
        let auth = test_auth(state_path.clone());
        let exporter = OtlpExporter::new("https://ingress.example.test:4317", Some(&auth)).unwrap();
        let provider = exporter.auth.as_ref().unwrap();
        *provider.cache.lock().await = Some(CachedToken {
            value: "cached-token".to_string(),
            refresh_at: Instant::now() + Duration::from_secs(60),
        });
        let logical_export_sequence = exporter.sequence.next().await.unwrap();
        let persisted_after_allocation = fs::read_to_string(&state_path).unwrap();

        let first_attempt = exporter.request((), logical_export_sequence).await.unwrap();
        let retry_attempt = exporter.request((), logical_export_sequence).await.unwrap();

        for request in [&first_attempt, &retry_attempt] {
            assert_eq!(
                request
                    .metadata()
                    .get("x-skid-sequence")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                logical_export_sequence.to_string()
            );
        }
        assert_eq!(
            fs::read_to_string(&state_path).unwrap(),
            persisted_after_allocation,
            "constructing retry requests must not allocate or persist again"
        );
    }

    #[test]
    fn retry_classification_is_bounded_to_transient_and_one_auth_refresh() {
        for code in [
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::Unknown,
            Code::ResourceExhausted,
            Code::Aborted,
        ] {
            assert_eq!(
                retry_decision(code, false, false),
                RetryDecision::Retry,
                "{code:?} should be transient"
            );
        }

        assert_eq!(
            retry_decision(Code::Unauthenticated, true, false),
            RetryDecision::RefreshAuth
        );
        assert_eq!(
            retry_decision(Code::Unauthenticated, true, true),
            RetryDecision::Stop
        );
        assert_eq!(
            retry_decision(Code::Unauthenticated, false, false),
            RetryDecision::Stop
        );
        for code in [Code::PermissionDenied, Code::InvalidArgument] {
            assert_eq!(
                retry_decision(code, true, false),
                RetryDecision::Stop,
                "{code:?} must not be retried"
            );
        }
    }

    #[test]
    fn retry_backoff_is_small_and_bounded() {
        assert_eq!(retry_backoff(1), Duration::from_millis(100));
        assert_eq!(retry_backoff(2), Duration::from_millis(200));
        assert_eq!(retry_backoff(100), Duration::from_millis(400));
    }

    #[tokio::test]
    async fn cached_token_is_attached_without_contacting_keycloak() {
        let directory = TestDirectory::new("token");
        let auth = test_auth(directory.state_path());
        let exporter = OtlpExporter::new("https://ingress.example.test:4317", Some(&auth)).unwrap();
        let provider = exporter.auth.as_ref().unwrap();
        *provider.cache.lock().await = Some(CachedToken {
            value: "cached-token".to_string(),
            refresh_at: Instant::now() + Duration::from_secs(60),
        });

        let request = exporter.request((), 7).await.unwrap();
        assert_eq!(
            request
                .metadata()
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer cached-token"
        );
        assert!(
            request
                .metadata()
                .get("authorization")
                .unwrap()
                .is_sensitive()
        );
    }
}
