use skid_monitor_core::{SignalRecord, TenantId};
use skid_protocol::protocol::Signal;
use std::str::FromStr;
use url::{Host, Url};
use uuid::Uuid;

#[cfg(target_arch = "wasm32")]
pub(super) const OIDC_ACCESS_TOKEN_SESSION_KEY: &str = "skid-monitor.oidc.access_token";
#[cfg(target_arch = "wasm32")]
pub(super) const LEGACY_KEYCLOAK_ACCESS_TOKEN_SESSION_KEY: &str =
    "skid-monitor.keycloak.access_token";
const CURSOR_STORAGE_PREFIX: &str = "skid-monitor.cloud.cursor.v1:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BrowserEndpoint {
    RawWebSocket(String),
    CloudApi(String),
}

impl BrowserEndpoint {
    #[cfg(target_arch = "wasm32")]
    pub(super) fn key(&self) -> &str {
        match self {
            Self::RawWebSocket(endpoint) | Self::CloudApi(endpoint) => endpoint,
        }
    }
}

pub(super) struct DecodedJsonSignal {
    pub(super) signal: Signal,
    pub(super) cursor: Option<u64>,
    pub(super) tenant_id: Option<TenantId>,
}

pub(super) fn normalize_endpoint(value: &str) -> Result<BrowserEndpoint, String> {
    let value = value.trim();
    let mut url = Url::parse(value).map_err(|_| {
        "browser ingress must be an http(s) client API or ws(s) bridge URL".to_string()
    })?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err("browser ingress URLs must not contain credentials".to_string());
    }

    match url.scheme() {
        "ws" | "wss" => Ok(BrowserEndpoint::RawWebSocket(url.to_string())),
        "http" | "https" => {
            if url.query().is_some() || url.fragment().is_some() {
                return Err(
                    "cloud client API URLs must not contain a query or fragment".to_string()
                );
            }
            if url.scheme() == "http" && !is_numeric_loopback(&url) {
                return Err(
                    "cloud client API must use https; http is allowed only on a numeric loopback address"
                        .to_string(),
                );
            }
            let normalized_path = url.path().trim_end_matches('/').to_string();
            url.set_path(if normalized_path.is_empty() {
                "/"
            } else {
                &normalized_path
            });
            Ok(BrowserEndpoint::CloudApi(url.to_string()))
        }
        _ => Err("browser ingress must be an http(s) client API or ws(s) bridge URL".to_string()),
    }
}

fn is_numeric_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(_)) | None => false,
    }
}

pub(super) fn cursor_storage_key(cloud_api: &str, tenant_id: TenantId) -> String {
    format!("{CURSOR_STORAGE_PREFIX}{cloud_api}:tenant:{tenant_id}")
}

pub(super) fn browser_storage_key(base: &str, cloud_api: &str, tenant_id: TenantId) -> String {
    format!("{base}:cloud:{cloud_api}:tenant:{tenant_id}")
}

/// Extracts the tenant identity from the server's `<tenant UUID>.<ticket UUID>`
/// value while validating the complete ticket shape. The opaque ticket UUID is
/// deliberately discarded and is never used as persistence namespace data.
pub(super) fn tenant_from_stream_ticket(value: &str) -> Result<TenantId, String> {
    let (tenant, ticket) = value
        .split_once('.')
        .ok_or_else(|| "stream-ticket response contains an invalid ticket".to_string())?;
    if ticket.contains('.') {
        return Err("stream-ticket response contains an invalid ticket".to_string());
    }
    let tenant_id = TenantId::from_str(tenant)
        .map_err(|_| "stream-ticket response contains an invalid tenant".to_string())?;
    let ticket_id = Uuid::parse_str(ticket)
        .map_err(|_| "stream-ticket response contains an invalid ticket".to_string())?;
    if tenant_id.to_string() != tenant || ticket_id.to_string() != ticket {
        return Err("stream-ticket response must use canonical UUIDs".to_string());
    }
    Ok(tenant_id)
}

pub(super) fn ticket_endpoint(cloud_api: &str) -> Result<String, String> {
    cloud_url(cloud_api, "v1/stream-tickets", false, None, 0)
}

pub(super) fn stream_endpoint(cloud_api: &str, ticket: &str, after: u64) -> Result<String, String> {
    cloud_url(cloud_api, "v1/stream", true, Some(ticket), after)
}

fn cloud_url(
    cloud_api: &str,
    suffix: &str,
    websocket: bool,
    ticket: Option<&str>,
    after: u64,
) -> Result<String, String> {
    let mut url = Url::parse(cloud_api).map_err(|_| "invalid cloud client API URL".to_string())?;
    let root = url.path().trim_end_matches('/');
    let path = if root.is_empty() {
        format!("/{suffix}")
    } else {
        format!("{root}/{suffix}")
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);

    if websocket {
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            _ => return Err("cloud client API URL must use http or https".to_string()),
        };
        url.set_scheme(scheme)
            .map_err(|_| "failed to build cloud stream URL".to_string())?;
        let ticket = ticket.ok_or_else(|| "cloud stream ticket is missing".to_string())?;
        url.query_pairs_mut()
            .append_pair("ticket", ticket)
            .append_pair("after", &after.to_string());
    }

    Ok(url.to_string())
}

pub(super) fn decode_json_signal(value: &str) -> Result<DecodedJsonSignal, String> {
    if let Ok(record) = serde_json::from_str::<SignalRecord>(value) {
        if record.cursor.0 == 0 {
            return Err("cloud signal record has an invalid zero cursor".to_string());
        }
        return Ok(DecodedJsonSignal {
            tenant_id: record.envelope.scope.tenant_id(),
            signal: record.envelope.payload,
            cursor: Some(record.cursor.0),
        });
    }

    serde_json::from_str(value)
        .map(|signal| DecodedJsonSignal {
            signal,
            cursor: None,
            tenant_id: None,
        })
        .map_err(|error| format!("invalid WebSocket signal JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skid_monitor_core::{AgentId, SignalCursor, SignalEnvelope, SignalScope};
    use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};

    fn signal() -> Signal {
        Signal::Metrics(export_metrics(
            vec![Metric {
                name: "system.cpu.usage".to_string(),
                value: 12.5,
                source: Source::System,
                unit: Some("%".to_string()),
                kind: MetricKind::Gauge,
                attributes: Vec::new(),
            }],
            "agent",
            "test",
        ))
    }

    #[test]
    fn endpoint_modes_are_explicit_and_cloud_base_is_canonical() {
        assert_eq!(
            normalize_endpoint(" https://monitor.example/client/ ").unwrap(),
            BrowserEndpoint::CloudApi("https://monitor.example/client".to_string())
        );
        assert_eq!(
            normalize_endpoint("wss://monitor.example/raw?format=json").unwrap(),
            BrowserEndpoint::RawWebSocket("wss://monitor.example/raw?format=json".to_string())
        );
        assert!(normalize_endpoint("https://token@example.com").is_err());
        assert!(normalize_endpoint("https://monitor.example?token=secret").is_err());
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:8080").unwrap(),
            BrowserEndpoint::CloudApi("http://127.0.0.1:8080/".to_string())
        );
        assert!(normalize_endpoint("http://monitor.example").is_err());
        assert!(normalize_endpoint("http://localhost:8080").is_err());
    }

    #[test]
    fn cloud_urls_keep_path_prefix_and_encode_ticket() {
        assert_eq!(
            ticket_endpoint("https://monitor.example/control").unwrap(),
            "https://monitor.example/control/v1/stream-tickets"
        );
        assert_eq!(
            stream_endpoint("https://monitor.example/control", "a ticket/+", 42).unwrap(),
            "wss://monitor.example/control/v1/stream?ticket=a+ticket%2F%2B&after=42"
        );
    }

    #[test]
    fn cursor_key_is_scoped_to_the_canonical_cloud_endpoint() {
        let tenant_id = TenantId::from_str("4d6f5ef3-f18d-4930-a3c8-13f013c9a004").unwrap();
        let other_tenant = TenantId::from_str("f2810043-7443-48ad-a418-20ed55f71a2a").unwrap();
        assert_eq!(
            cursor_storage_key("https://monitor.example/control", tenant_id),
            "skid-monitor.cloud.cursor.v1:https://monitor.example/control:tenant:4d6f5ef3-f18d-4930-a3c8-13f013c9a004"
        );
        assert_eq!(
            browser_storage_key(
                "skid-monitor.edge-state.v1",
                "https://monitor.example/control",
                tenant_id,
            ),
            "skid-monitor.edge-state.v1:cloud:https://monitor.example/control:tenant:4d6f5ef3-f18d-4930-a3c8-13f013c9a004"
        );
        assert_ne!(
            cursor_storage_key("https://monitor.example/control", tenant_id),
            cursor_storage_key("https://monitor.example/control", other_tenant)
        );
        assert_ne!(
            browser_storage_key(
                "skid-monitor.edge-state.v1",
                "https://monitor.example/control",
                tenant_id,
            ),
            browser_storage_key(
                "skid-monitor.edge-state.v1",
                "https://monitor.example/control",
                other_tenant,
            )
        );
    }

    #[test]
    fn stream_ticket_proves_a_canonical_tenant_without_persisting_ticket_id() {
        let tenant = "4d6f5ef3-f18d-4930-a3c8-13f013c9a004";
        let ticket = "df783721-1280-4e7d-a6e0-977f25845fc1";
        assert_eq!(
            tenant_from_stream_ticket(&format!("{tenant}.{ticket}"))
                .unwrap()
                .to_string(),
            tenant
        );
        assert!(tenant_from_stream_ticket("not-a-tenant.not-a-ticket").is_err());
        assert!(tenant_from_stream_ticket(&format!("{tenant}.{ticket}.extra")).is_err());
        assert!(
            tenant_from_stream_ticket(&format!("{}.{}", tenant.replace('-', ""), ticket)).is_err()
        );
    }

    #[test]
    fn full_cloud_record_and_legacy_raw_signal_are_distinguished() {
        let tenant_id = TenantId::from_str("4d6f5ef3-f18d-4930-a3c8-13f013c9a004").unwrap();
        let record = SignalRecord {
            cursor: SignalCursor(73),
            envelope: SignalEnvelope::new(
                SignalScope::tenant(tenant_id),
                AgentId::new("edge-a").unwrap(),
                2,
                3,
                signal(),
            ),
        };
        let decoded = decode_json_signal(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(decoded.cursor, Some(73));
        assert_eq!(decoded.tenant_id, Some(tenant_id));
        assert!(matches!(decoded.signal, Signal::Metrics(_)));

        let decoded = decode_json_signal(&serde_json::to_string(&signal()).unwrap()).unwrap();
        assert_eq!(decoded.cursor, None);
        assert_eq!(decoded.tenant_id, None);
        assert!(matches!(decoded.signal, Signal::Metrics(_)));
    }
}
