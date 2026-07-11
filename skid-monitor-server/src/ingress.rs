//! Keycloak-authenticated OTLP ingress for the cloud deployment.
//!
//! This plane accepts agent credentials only. It commits each canonical
//! [`Signal`] to PostgreSQL before returning the OTLP acknowledgement, leaving
//! client reads and live streaming to the separate client-access plane.

use crate::auth::{AuthError, JwtVerifier};
use crate::config::{IngressConfig, TlsMode};
use crate::store::{PgSignalStore, PgStoreError, PgStoreOptions};
use skid_monitor_core::{AgentId, SignalEnvelope, SignalScope, SignalWriter, TenantId};
use skid_protocol::otlp::tonic::collector::logs::v1::ExportLogsServiceResponse;
use skid_protocol::otlp::tonic::collector::logs::v1::logs_service_server::{
    LogsService, LogsServiceServer,
};
use skid_protocol::otlp::tonic::collector::metrics::v1::ExportMetricsServiceResponse;
use skid_protocol::otlp::tonic::collector::metrics::v1::metrics_service_server::{
    MetricsService, MetricsServiceServer,
};
use skid_protocol::otlp::tonic::collector::trace::v1::ExportTraceServiceResponse;
use skid_protocol::otlp::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use skid_protocol::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use skid_protocol::protocol::Signal;
use std::error::Error;
use std::io;
use std::path::Path;
use tonic::metadata::MetadataMap;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use tower::limit::GlobalConcurrencyLimitLayer;
use tracing::{info, warn};

const AUTHORIZATION_METADATA: &str = "authorization";
const SEQUENCE_METADATA: &str = "x-skid-sequence";

#[derive(Clone)]
struct CloudOtlpIngest {
    store: PgSignalStore,
    verifier: JwtVerifier,
}

#[derive(Debug)]
struct IngestContext {
    tenant_id: TenantId,
    agent_id: AgentId,
    sequence: u64,
}

/// Starts the cloud agent-ingress plane.
///
/// `TerminatedUpstream` deliberately serves plaintext on the configured
/// address and must only be used behind the trusted TLS proxy/service mesh
/// explicitly selected by [`IngressConfig`].
pub async fn serve(config: IngressConfig) -> Result<(), Box<dyn Error + Send + Sync>> {
    let identity = load_tls_identity(&config.tls).await?;

    let store = PgSignalStore::connect(
        &config.database.url,
        PgStoreOptions {
            max_connections: config.database.max_connections,
            ..PgStoreOptions::default()
        },
    )
    .await?;
    store.verify_ready().await?;

    let verifier = JwtVerifier::discover(config.jwt.clone()).await?;
    let ingest = CloudOtlpIngest { store, verifier };

    let metrics = MetricsServiceServer::new(ingest.clone())
        .max_decoding_message_size(config.max_signal_bytes);
    let traces =
        TraceServiceServer::new(ingest.clone()).max_decoding_message_size(config.max_signal_bytes);
    let logs = LogsServiceServer::new(ingest).max_decoding_message_size(config.max_signal_bytes);

    // This shared layer wraps the raw HTTP request router, before tonic invokes
    // any generated service and decodes its protobuf body. Unlike the
    // per-connection guard below, its semaphore is shared across every HTTP/2
    // connection and all three OTLP signal services.
    let mut server = Server::builder()
        .concurrency_limit_per_connection(config.concurrency_per_connection)
        .layer(GlobalConcurrencyLimitLayer::new(
            config.global_request_concurrency,
        ));
    if let Some(identity) = identity {
        // tonic and the direct rustls dependency are both compiled with the
        // ring provider. Installing it explicitly avoids provider ambiguity
        // when another dependency also enables a rustls backend.
        let _ = rustls::crypto::ring::default_provider().install_default();
        server = server.tls_config(ServerTlsConfig::new().identity(identity))?;
        info!(addr = %config.listen_addr, "cloud OTLP ingress listening with direct TLS");
    } else {
        info!(
            addr = %config.listen_addr,
            "cloud OTLP ingress listening behind trusted upstream TLS termination"
        );
    }

    server
        .add_service(metrics)
        .add_service(traces)
        .add_service(logs)
        .serve(config.listen_addr)
        .await?;
    Ok(())
}

impl CloudOtlpIngest {
    async fn context(&self, metadata: &MetadataMap) -> Result<IngestContext, Status> {
        // Copy the header before awaiting so request metadata is never held
        // across the OIDC verification/possible JWKS refresh.
        let authorization = required_metadata(metadata, AUTHORIZATION_METADATA)?.to_string();
        let principal = self
            .verifier
            .verify_bearer(&authorization)
            .await
            .map_err(auth_status)?;
        let agent_id = principal.agent_id().map_err(auth_status)?;
        let sequence = sequence_from_metadata(metadata)?;
        Ok(IngestContext {
            tenant_id: principal.tenant_id,
            agent_id,
            sequence,
        })
    }

    async fn append(&self, context: IngestContext, payload: Signal) -> Result<(), Status> {
        let tenant_id = context.tenant_id;
        let agent_id = context.agent_id;
        let sequence = context.sequence;
        let envelope = SignalEnvelope::now(
            SignalScope::tenant(tenant_id),
            agent_id.clone(),
            sequence,
            payload,
        );

        // SignalWriter resolves only after PgSignalStore commits its
        // transaction. Returning success below is therefore the durable ACK.
        let outcome = self.store.append(&envelope).await.map_err(store_status)?;
        info!(
            tenant_id = %tenant_id,
            agent_id = %agent_id,
            sequence,
            cursor = outcome.cursor.0,
            inserted = outcome.inserted,
            signal_kind = ?envelope.kind(),
            "committed cloud OTLP signal"
        );
        Ok(())
    }
}

#[tonic::async_trait]
impl MetricsService for CloudOtlpIngest {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let context = self.context(request.metadata()).await?;
        self.append(context, Signal::Metrics(request.into_inner()))
            .await?;
        Ok(Response::new(ExportMetricsServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl TraceService for CloudOtlpIngest {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let context = self.context(request.metadata()).await?;
        self.append(context, Signal::Traces(request.into_inner()))
            .await?;
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: None,
        }))
    }
}

#[tonic::async_trait]
impl LogsService for CloudOtlpIngest {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let context = self.context(request.metadata()).await?;
        self.append(context, Signal::Logs(request.into_inner()))
            .await?;
        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: None,
        }))
    }
}

fn required_metadata<'a>(metadata: &'a MetadataMap, name: &'static str) -> Result<&'a str, Status> {
    metadata
        .get(name)
        .ok_or_else(|| {
            if name == AUTHORIZATION_METADATA {
                Status::unauthenticated("agent credentials are required")
            } else {
                Status::invalid_argument(format!("required metadata {name} is missing"))
            }
        })?
        .to_str()
        .map_err(|_| {
            if name == AUTHORIZATION_METADATA {
                Status::unauthenticated("agent credentials are invalid")
            } else {
                Status::invalid_argument(format!("metadata {name} is not valid ASCII"))
            }
        })
}

fn sequence_from_metadata(metadata: &MetadataMap) -> Result<u64, Status> {
    let value = required_metadata(metadata, SEQUENCE_METADATA)?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Status::invalid_argument(
            "x-skid-sequence must be an unsigned decimal integer",
        ));
    }
    value
        .parse::<u64>()
        .map_err(|_| Status::invalid_argument("x-skid-sequence is outside the supported u64 range"))
}

fn auth_status(error: AuthError) -> Status {
    match error {
        AuthError::MissingRole(_) => {
            warn!(reason = %error, "agent token lacks ingress permission");
            Status::permission_denied("agent is not authorized for telemetry ingest")
        }
        AuthError::Discovery(_) | AuthError::KeySet(_) => {
            warn!(reason = %error, "agent authentication backend is unavailable");
            Status::unavailable("authentication service is unavailable")
        }
        AuthError::MissingBearer
        | AuthError::InvalidBearer
        | AuthError::InvalidToken(_)
        | AuthError::InvalidClaims(_) => {
            warn!(reason = %error, "agent authentication failed");
            Status::unauthenticated("agent credentials are invalid")
        }
    }
}

fn store_status(error: PgStoreError) -> Status {
    match error {
        PgStoreError::TenantDisabled { .. }
        | PgStoreError::AgentNotEnrolled { .. }
        | PgStoreError::AgentDisabled { .. } => {
            warn!(reason = %error, "agent enrollment rejected telemetry");
            // Do not disclose whether a named agent exists or was disabled.
            Status::permission_denied("tenant and agent must be enrolled and enabled")
        }
        PgStoreError::IntegerOutOfRange { field: "sequence" } => {
            Status::invalid_argument("x-skid-sequence exceeds the supported range")
        }
        PgStoreError::InvalidSignalPayload(_)
        | PgStoreError::SignalPayloadNotRoundTripSafe
        | PgStoreError::SignalPayloadTooLarge { .. } => {
            warn!(reason = %error, "agent submitted an unsupported signal payload");
            Status::invalid_argument(
                "signal payload must be JSON round-trip safe and no larger than 16 MiB",
            )
        }
        PgStoreError::SequenceConflict { .. } => {
            warn!(reason = %error, "agent reused a signal sequence");
            Status::already_exists("x-skid-sequence was already used for another signal")
        }
        PgStoreError::Database(_) => {
            warn!(reason = %error, "signal store is unavailable");
            Status::unavailable("signal store is unavailable")
        }
        _ => {
            warn!(reason = %error, "signal commit failed");
            Status::internal("signal could not be committed")
        }
    }
}

async fn load_tls_identity(mode: &TlsMode) -> Result<Option<Identity>, io::Error> {
    let TlsMode::Direct { cert, key } = mode else {
        return Ok(None);
    };
    let (cert_pem, key_pem) = tokio::try_join!(read_tls_file(cert), read_tls_file(key))?;
    Ok(Some(Identity::from_pem(cert_pem, key_pem)))
}

async fn read_tls_file(path: &Path) -> Result<Vec<u8>, io::Error> {
    tokio::fs::read(path).await.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to read TLS file {}: {error}", path.display()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn sequence_metadata_is_required_and_strictly_decimal() {
        let empty = MetadataMap::new();
        assert_eq!(
            sequence_from_metadata(&empty).unwrap_err().code(),
            Code::InvalidArgument
        );

        for invalid in ["", "+1", "-1", " 1", "1 ", "0x10", "1.0"] {
            let mut metadata = MetadataMap::new();
            metadata.insert(SEQUENCE_METADATA, invalid.parse().unwrap());
            assert_eq!(
                sequence_from_metadata(&metadata).unwrap_err().code(),
                Code::InvalidArgument,
                "{invalid:?} should not be accepted"
            );
        }
    }

    #[test]
    fn sequence_metadata_accepts_full_u64_domain() {
        let mut metadata = MetadataMap::new();
        metadata.insert(SEQUENCE_METADATA, "0".parse().unwrap());
        assert_eq!(sequence_from_metadata(&metadata).unwrap(), 0);

        metadata.insert(SEQUENCE_METADATA, u64::MAX.to_string().parse().unwrap());
        assert_eq!(sequence_from_metadata(&metadata).unwrap(), u64::MAX);
    }

    #[test]
    fn authorization_metadata_failure_is_unauthenticated() {
        let metadata = MetadataMap::new();
        assert_eq!(
            required_metadata(&metadata, AUTHORIZATION_METADATA)
                .unwrap_err()
                .code(),
            Code::Unauthenticated
        );
    }

    #[test]
    fn unsafe_or_oversized_signal_payload_is_not_retried_as_a_store_outage() {
        assert_eq!(
            store_status(PgStoreError::SignalPayloadNotRoundTripSafe).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            store_status(PgStoreError::SignalPayloadTooLarge {
                actual_bytes: 16 * 1024 * 1024 + 1,
                max_bytes: 16 * 1024 * 1024,
            })
            .code(),
            Code::InvalidArgument
        );
    }
}
