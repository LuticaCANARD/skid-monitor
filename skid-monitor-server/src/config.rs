use std::env;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::path::PathBuf;

const DEFAULT_DATABASE_CONNECTIONS: u32 = 12;
const DEFAULT_MAX_SIGNAL_BYTES: usize = 16 * 1024 * 1024;
const MAX_SIGNAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_CLIENT_BODY_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_STREAM_BATCH_BYTES: usize = 16 * 1024 * 1024;
const MAX_STREAM_BATCH_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_INGRESS_CONCURRENCY_PER_CONNECTION: usize = 64;
const DEFAULT_INGRESS_GLOBAL_REQUEST_CONCURRENCY: usize = 16;
const MAX_INGRESS_GLOBAL_REQUEST_CONCURRENCY: usize = 128;
const DEFAULT_CLIENT_REQUEST_CONCURRENCY: usize = 256;
const DEFAULT_CLIENT_STREAM_CONNECTIONS: usize = 1_024;
const DEFAULT_CLIENT_REPLAY_CONCURRENCY: usize = 4;
const MAX_CLIENT_REPLAY_CONCURRENCY: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OidcRoleClaims {
    KeycloakResourceAccess,
    ClaimPointer(String),
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer: String,
    pub jwks_origin: String,
    pub audience: String,
    pub required_role: String,
    pub tenant_pointer: String,
    pub role_claims: OidcRoleClaims,
    pub agent_id_pointer: Option<String>,
}

#[derive(Clone, Debug)]
pub enum TlsMode {
    Direct { cert: PathBuf, key: PathBuf },
    TerminatedUpstream,
}

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Clone, Debug)]
pub struct IngressConfig {
    pub listen_addr: SocketAddr,
    pub database: DatabaseConfig,
    pub oidc: OidcConfig,
    pub tls: TlsMode,
    pub max_signal_bytes: usize,
    pub concurrency_per_connection: usize,
    pub global_request_concurrency: usize,
}

#[derive(Clone, Debug)]
pub struct ClientServerConfig {
    pub listen_addr: SocketAddr,
    pub database: DatabaseConfig,
    pub oidc: OidcConfig,
    pub admin_role: String,
    pub tls: TlsMode,
    pub request_body_limit: usize,
    pub stream_batch_size: usize,
    pub stream_batch_bytes: usize,
    pub request_concurrency: usize,
    pub replay_concurrency: usize,
    pub max_stream_connections: usize,
}

#[derive(Clone, Debug)]
pub struct MigrationConfig {
    pub database: DatabaseConfig,
}

#[derive(Debug)]
pub struct ConfigError(String);

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl IngressConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            listen_addr: parse_socket_addr("SKID_MONITOR_INGRESS_ADDR", "0.0.0.0:4317")?,
            database: database_config()?,
            oidc: oidc_config(
                "SKID_MONITOR_INGRESS_AUDIENCE",
                "SKID_MONITOR_INGRESS_ROLE",
                "telemetry-ingest",
            )?,
            tls: tls_mode(
                "SKID_MONITOR_INGRESS_TLS_CERT",
                "SKID_MONITOR_INGRESS_TLS_KEY",
                "SKID_MONITOR_INGRESS_TLS_TERMINATED",
            )?,
            max_signal_bytes: parse_positive(
                "SKID_MONITOR_MAX_SIGNAL_BYTES",
                DEFAULT_MAX_SIGNAL_BYTES,
            )?
            .min(MAX_SIGNAL_BYTES),
            concurrency_per_connection: parse_positive(
                "SKID_MONITOR_INGRESS_CONCURRENCY_PER_CONNECTION",
                DEFAULT_INGRESS_CONCURRENCY_PER_CONNECTION,
            )?
            .min(4_096),
            global_request_concurrency: bounded_ingress_global_request_concurrency(parse_positive(
                "SKID_MONITOR_INGRESS_GLOBAL_REQUEST_CONCURRENCY",
                DEFAULT_INGRESS_GLOBAL_REQUEST_CONCURRENCY,
            )?),
        })
    }
}

impl ClientServerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            listen_addr: parse_socket_addr("SKID_MONITOR_CLIENT_SERVER_ADDR", "0.0.0.0:8080")?,
            database: database_config()?,
            oidc: oidc_config(
                "SKID_MONITOR_CLIENT_AUDIENCE",
                "SKID_MONITOR_CLIENT_READ_ROLE",
                "telemetry-read",
            )?,
            admin_role: env_non_empty("SKID_MONITOR_CLIENT_ADMIN_ROLE")
                .unwrap_or_else(|| "telemetry-admin".to_string()),
            tls: tls_mode(
                "SKID_MONITOR_CLIENT_TLS_CERT",
                "SKID_MONITOR_CLIENT_TLS_KEY",
                "SKID_MONITOR_CLIENT_TLS_TERMINATED",
            )?,
            request_body_limit: parse_positive("SKID_MONITOR_CLIENT_BODY_LIMIT", 1024 * 1024)?
                .min(MAX_CLIENT_BODY_BYTES),
            stream_batch_size: parse_positive("SKID_MONITOR_STREAM_BATCH_SIZE", 256)?.min(4096),
            stream_batch_bytes: parse_positive(
                "SKID_MONITOR_STREAM_BATCH_BYTES",
                DEFAULT_STREAM_BATCH_BYTES,
            )?
            .min(MAX_STREAM_BATCH_BYTES),
            request_concurrency: parse_positive(
                "SKID_MONITOR_CLIENT_REQUEST_CONCURRENCY",
                DEFAULT_CLIENT_REQUEST_CONCURRENCY,
            )?
            .min(16_384),
            replay_concurrency: bounded_replay_concurrency(parse_positive(
                "SKID_MONITOR_CLIENT_REPLAY_CONCURRENCY",
                DEFAULT_CLIENT_REPLAY_CONCURRENCY,
            )?),
            max_stream_connections: parse_positive(
                "SKID_MONITOR_CLIENT_MAX_STREAM_CONNECTIONS",
                DEFAULT_CLIENT_STREAM_CONNECTIONS,
            )?
            .min(65_536),
        })
    }
}

impl MigrationConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database: database_config_from(
                "SKID_MONITOR_MIGRATION_DATABASE_URL",
                "SKID_MONITOR_MIGRATION_DATABASE_MAX_CONNECTIONS",
                "SKID_MONITOR_MIGRATION_DATABASE_TLS_TERMINATED",
                2,
            )?,
        })
    }
}

fn database_config() -> Result<DatabaseConfig, ConfigError> {
    database_config_from(
        "SKID_MONITOR_DATABASE_URL",
        "SKID_MONITOR_DATABASE_MAX_CONNECTIONS",
        "SKID_MONITOR_DATABASE_TLS_TERMINATED",
        DEFAULT_DATABASE_CONNECTIONS,
    )
}

fn database_config_from(
    url_env: &str,
    max_connections_env: &str,
    tls_terminated_env: &str,
    default_connections: u32,
) -> Result<DatabaseConfig, ConfigError> {
    let url = required(url_env)?;
    let tls_terminated = parse_bool(tls_terminated_env, false)?;
    validate_database_url(&url, tls_terminated)?;
    Ok(DatabaseConfig {
        url,
        max_connections: parse_positive(max_connections_env, default_connections)?,
    })
}

fn validate_database_url(value: &str, tls_terminated: bool) -> Result<(), ConfigError> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        ConfigError("SKID_MONITOR_DATABASE_URL must be a valid PostgreSQL URL".to_string())
    })?;
    if !matches!(url.scheme(), "postgres" | "postgresql") || url.host_str().is_none() {
        return Err(ConfigError(
            "SKID_MONITOR_DATABASE_URL must use postgres:// or postgresql://".to_string(),
        ));
    }
    let sslmodes = url
        .query_pairs()
        .filter_map(|(key, value)| (key == "sslmode").then(|| value.into_owned()))
        .collect::<Vec<_>>();
    if sslmodes.len() > 1 {
        return Err(ConfigError(
            "SKID_MONITOR_DATABASE_URL must contain exactly one sslmode setting".to_string(),
        ));
    }
    let sslmode = sslmodes.first().map(String::as_str);
    match (sslmode, tls_terminated) {
        (Some("verify-full"), _) => Ok(()),
        (Some("disable"), true) => Ok(()),
        (Some("disable"), false) => Err(ConfigError(
            "PostgreSQL plaintext requires SKID_MONITOR_DATABASE_TLS_TERMINATED=true on a trusted private transport"
                .to_string(),
        )),
        (None, _) => Err(ConfigError(
            "SKID_MONITOR_DATABASE_URL must set sslmode=verify-full, or sslmode=disable with explicit trusted TLS termination"
                .to_string(),
        )),
        (Some(mode), _) => Err(ConfigError(format!(
            "PostgreSQL sslmode={mode} is not accepted; use verify-full, or explicit trusted TLS termination with disable"
        ))),
    }
}

fn oidc_config(
    audience_env: &str,
    role_env: &str,
    default_role: &str,
) -> Result<OidcConfig, ConfigError> {
    let issuer = env_non_empty("SKID_MONITOR_OIDC_ISSUER")
        .or_else(|| env_non_empty("SKID_MONITOR_KEYCLOAK_ISSUER"))
        .ok_or_else(|| {
            ConfigError(
                "required setting SKID_MONITOR_OIDC_ISSUER is missing (legacy SKID_MONITOR_KEYCLOAK_ISSUER is also accepted)"
                    .to_string(),
            )
        })?;
    let tenant_pointer = match env_non_empty("SKID_MONITOR_OIDC_TENANT_POINTER") {
        Some(pointer) => validated_claim_pointer("SKID_MONITOR_OIDC_TENANT_POINTER", &pointer)?,
        None => env_non_empty("SKID_MONITOR_TENANT_CLAIM")
            .map(|claim| top_level_claim_pointer(&claim))
            .unwrap_or_else(|| "/tenant_id".to_string()),
    };
    let role_claims = parse_oidc_role_claims(
        env_non_empty("SKID_MONITOR_OIDC_CLAIMS_PROFILE").as_deref(),
        env_non_empty("SKID_MONITOR_OIDC_ROLES_POINTER").as_deref(),
    )?;
    let agent_id_pointer = env_non_empty("SKID_MONITOR_OIDC_AGENT_ID_POINTER")
        .map(|pointer| validated_claim_pointer("SKID_MONITOR_OIDC_AGENT_ID_POINTER", &pointer))
        .transpose()?;
    let issuer = normalized_https_issuer(&issuer)?;
    let jwks_origin = match env_non_empty("SKID_MONITOR_OIDC_JWKS_ORIGIN") {
        Some(origin) => normalized_https_origin("SKID_MONITOR_OIDC_JWKS_ORIGIN", &origin)?,
        None => https_origin("SKID_MONITOR_OIDC_ISSUER", &issuer)?,
    };
    Ok(OidcConfig {
        issuer,
        jwks_origin,
        audience: required(audience_env)?,
        required_role: env_non_empty(role_env).unwrap_or_else(|| default_role.to_string()),
        tenant_pointer,
        role_claims,
        agent_id_pointer,
    })
}

fn parse_oidc_role_claims(
    profile: Option<&str>,
    roles_pointer: Option<&str>,
) -> Result<OidcRoleClaims, ConfigError> {
    match profile
        .unwrap_or("keycloak")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "keycloak" => {
            if roles_pointer.is_some() {
                return Err(ConfigError(
                    "SKID_MONITOR_OIDC_ROLES_POINTER requires SKID_MONITOR_OIDC_CLAIMS_PROFILE=generic"
                        .to_string(),
                ));
            }
            Ok(OidcRoleClaims::KeycloakResourceAccess)
        }
        "generic" => Ok(OidcRoleClaims::ClaimPointer(validated_claim_pointer(
            "SKID_MONITOR_OIDC_ROLES_POINTER",
            roles_pointer.unwrap_or("/roles"),
        )?)),
        _ => Err(ConfigError(
            "SKID_MONITOR_OIDC_CLAIMS_PROFILE must be keycloak or generic".to_string(),
        )),
    }
}

fn validated_claim_pointer(name: &str, pointer: &str) -> Result<String, ConfigError> {
    let pointer = pointer.trim();
    if pointer.is_empty()
        || !pointer.starts_with('/')
        || pointer.len() > 1_024
        || pointer.chars().any(char::is_control)
        || pointer
            .split('~')
            .skip(1)
            .any(|suffix| !matches!(suffix.chars().next(), Some('0' | '1')))
    {
        return Err(ConfigError(format!(
            "{name} must be a non-empty RFC 6901 JSON pointer using only ~0 and ~1 escapes"
        )));
    }
    Ok(pointer.to_string())
}

fn top_level_claim_pointer(claim: &str) -> String {
    format!("/{}", claim.replace('~', "~0").replace('/', "~1"))
}

fn normalized_https_issuer(value: &str) -> Result<String, ConfigError> {
    let issuer = value.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(issuer).map_err(|_| {
        ConfigError("SKID_MONITOR_OIDC_ISSUER must be a valid HTTPS URL".to_string())
    })?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ConfigError(
            "SKID_MONITOR_OIDC_ISSUER must be an HTTPS URL without credentials, query, or fragment"
                .to_string(),
        ));
    }
    Ok(issuer.to_string())
}

fn normalized_https_origin(name: &str, value: &str) -> Result<String, ConfigError> {
    let value = value.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| ConfigError(format!("{name} must be a valid HTTPS origin")))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ConfigError(format!(
            "{name} must contain only an HTTPS scheme, host, and optional port"
        )));
    }
    Ok(parsed.origin().ascii_serialization())
}

fn https_origin(name: &str, value: &str) -> Result<String, ConfigError> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|_| ConfigError(format!("{name} must be a valid HTTPS URL")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ConfigError(format!("{name} must use HTTPS")));
    }
    Ok(parsed.origin().ascii_serialization())
}

fn tls_mode(cert_env: &str, key_env: &str, terminated_env: &str) -> Result<TlsMode, ConfigError> {
    let cert = env_non_empty(cert_env);
    let key = env_non_empty(key_env);
    match (cert, key) {
        (Some(cert), Some(key)) => Ok(TlsMode::Direct {
            cert: PathBuf::from(cert),
            key: PathBuf::from(key),
        }),
        (None, None) if parse_bool(terminated_env, false)? => Ok(TlsMode::TerminatedUpstream),
        (None, None) => Err(ConfigError(format!(
            "cloud mode requires {cert_env}+{key_env}, or an explicit {terminated_env}=true behind a trusted TLS proxy/service mesh"
        ))),
        _ => Err(ConfigError(format!(
            "{cert_env} and {key_env} must be configured together"
        ))),
    }
}

fn required(name: &str) -> Result<String, ConfigError> {
    env_non_empty(name).ok_or_else(|| ConfigError(format!("required setting {name} is missing")))
}

fn env_non_empty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_socket_addr(name: &str, default: &str) -> Result<SocketAddr, ConfigError> {
    env_non_empty(name)
        .unwrap_or_else(|| default.to_string())
        .parse()
        .map_err(|error| ConfigError(format!("invalid {name}: {error}")))
}

fn parse_positive<T>(name: &str, default: T) -> Result<T, ConfigError>
where
    T: Copy + Display + FromEnvValue + PartialEq,
{
    let value = match env_non_empty(name) {
        Some(value) => T::from_env_value(&value)
            .map_err(|error| ConfigError(format!("invalid {name}: {error}")))?,
        None => default,
    };
    if value == T::zero() {
        return Err(ConfigError(format!("{name} must be greater than zero")));
    }
    Ok(value)
}

trait FromEnvValue: Sized {
    fn from_env_value(value: &str) -> Result<Self, String>;
    fn zero() -> Self;
}

impl FromEnvValue for usize {
    fn from_env_value(value: &str) -> Result<Self, String> {
        value.parse::<usize>().map_err(|error| error.to_string())
    }

    fn zero() -> Self {
        0
    }
}

impl FromEnvValue for u32 {
    fn from_env_value(value: &str) -> Result<Self, String> {
        value.parse::<u32>().map_err(|error| error.to_string())
    }

    fn zero() -> Self {
        0
    }
}

fn parse_bool(name: &str, default: bool) -> Result<bool, ConfigError> {
    let Some(value) = env_non_empty(name) else {
        return Ok(default);
    };
    parse_bool_value(name, &value)
}

fn parse_bool_value(name: &str, value: &str) -> Result<bool, ConfigError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError(format!("{name} must be true or false"))),
    }
}

fn bounded_replay_concurrency(value: usize) -> usize {
    value.min(MAX_CLIENT_REPLAY_CONCURRENCY)
}

fn bounded_ingress_global_request_concurrency(value: usize) -> usize {
    value.min(MAX_INGRESS_GLOBAL_REQUEST_CONCURRENCY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_parser_is_fail_closed() {
        for value in ["true", "1", "yes", "on"] {
            assert!(parse_bool_value("TEST", value).unwrap());
        }
        for value in ["false", "0", "no", "off"] {
            assert!(!parse_bool_value("TEST", value).unwrap());
        }
        assert!(parse_bool_value("TEST", "maybe").is_err());
    }

    #[test]
    fn direct_tls_requires_both_files() {
        let direct = TlsMode::Direct {
            cert: PathBuf::from("cert.pem"),
            key: PathBuf::from("key.pem"),
        };
        assert!(matches!(direct, TlsMode::Direct { .. }));
    }

    #[test]
    fn oidc_issuer_rejects_url_confusion() {
        assert_eq!(
            normalized_https_issuer("https://id.example/realms/skid/").unwrap(),
            "https://id.example/realms/skid"
        );
        for invalid in [
            "http://id.example/realms/skid",
            "https://user@id.example/realms/skid",
            "https://id.example/realms/skid?next=https://evil.example",
            "https://id.example/realms/skid#fragment",
        ] {
            assert!(normalized_https_issuer(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn oidc_jwks_origin_is_https_and_pathless() {
        assert_eq!(
            normalized_https_origin("TEST", "https://keys.example:8443/").unwrap(),
            "https://keys.example:8443"
        );
        for invalid in [
            "http://keys.example",
            "https://user@keys.example",
            "https://keys.example/jwks",
            "https://keys.example?tenant=a",
        ] {
            assert!(
                normalized_https_origin("TEST", invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn oidc_claim_profiles_are_explicit_and_pointer_safe() {
        assert_eq!(
            parse_oidc_role_claims(None, None).unwrap(),
            OidcRoleClaims::KeycloakResourceAccess
        );
        assert_eq!(
            parse_oidc_role_claims(Some("generic"), Some("/https:~1~1example.com~1roles")).unwrap(),
            OidcRoleClaims::ClaimPointer("/https:~1~1example.com~1roles".to_string())
        );
        assert!(parse_oidc_role_claims(Some("keycloak"), Some("/roles")).is_err());
        assert!(parse_oidc_role_claims(Some("unknown"), None).is_err());
        assert!(validated_claim_pointer("TEST", "roles").is_err());
        assert!(validated_claim_pointer("TEST", "/roles/~2invalid").is_err());
        assert_eq!(top_level_claim_pointer("org/tenant~id"), "/org~1tenant~0id");
    }

    #[test]
    fn cloud_database_tls_is_fail_closed() {
        assert!(
            validate_database_url("postgresql://db.example/skid?sslmode=verify-full", false)
                .is_ok()
        );
        assert!(validate_database_url("postgresql://postgres/skid?sslmode=disable", true).is_ok());
        for url in [
            "postgresql://db.example/skid",
            "postgresql://db.example/skid?sslmode=prefer",
            "postgresql://db.example/skid?sslmode=require",
            "postgresql://db.example/skid?sslmode=disable",
            "postgresql://db.example/skid?sslmode=verify-full&sslmode=disable",
        ] {
            assert!(validate_database_url(url, false).is_err(), "{url}");
        }
    }

    #[test]
    fn replay_concurrency_has_a_conservative_default_and_hard_cap() {
        assert_eq!(DEFAULT_CLIENT_REPLAY_CONCURRENCY, 4);
        assert_eq!(bounded_replay_concurrency(1), 1);
        assert_eq!(bounded_replay_concurrency(8), 8);
        assert_eq!(
            bounded_replay_concurrency(usize::MAX),
            MAX_CLIENT_REPLAY_CONCURRENCY
        );
    }

    #[test]
    fn ingress_global_request_concurrency_has_a_conservative_default_and_hard_cap() {
        assert_eq!(DEFAULT_INGRESS_GLOBAL_REQUEST_CONCURRENCY, 16);
        assert_eq!(bounded_ingress_global_request_concurrency(1), 1);
        assert_eq!(bounded_ingress_global_request_concurrency(64), 64);
        assert_eq!(
            bounded_ingress_global_request_concurrency(usize::MAX),
            MAX_INGRESS_GLOBAL_REQUEST_CONCURRENCY
        );
    }
}
