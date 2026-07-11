use crate::config::{OidcConfig, OidcRoleClaims};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use skid_monitor_core::{AgentId, TenantId};
use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const JWKS_REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_AUTHORIZATION_UNIX: u64 = 253_402_300_799;
const MAX_ACCESS_TOKEN_LIFETIME: u64 = 24 * 60 * 60;
const TOKEN_CLOCK_SKEW: u64 = 30;
const MAX_DISCOVERY_BYTES: usize = 64 * 1024;
const MAX_JWKS_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AuthenticatedPrincipal {
    pub subject: String,
    pub agent_identity: Option<String>,
    pub tenant_id: TenantId,
    pub roles: BTreeSet<String>,
    pub preferred_username: Option<String>,
    pub authorized_until_unix: u64,
}

impl AuthenticatedPrincipal {
    pub fn agent_id(&self) -> Result<AgentId, AuthError> {
        let identity = self.agent_identity.as_deref().ok_or_else(|| {
            AuthError::InvalidClaims(
                "agent identity claim is missing; configure SKID_MONITOR_OIDC_AGENT_ID_POINTER for this provider"
                    .to_string(),
            )
        })?;
        AgentId::new(identity).map_err(|error| AuthError::InvalidClaims(error.to_string()))
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(role)
    }
}

#[derive(Clone)]
pub struct OidcVerifier {
    config: OidcConfig,
    client: reqwest::Client,
    jwks_uri: String,
    keys: Arc<RwLock<JwkSet>>,
    last_jwks_refresh: Arc<Mutex<Instant>>,
}

#[derive(Debug)]
pub enum AuthError {
    MissingBearer,
    InvalidBearer,
    Discovery(String),
    KeySet(String),
    InvalidToken(String),
    InvalidClaims(String),
    MissingRole(String),
}

impl Display for AuthError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBearer => formatter.write_str("bearer token is required"),
            Self::InvalidBearer => formatter.write_str("authorization header is invalid"),
            Self::Discovery(error) => write!(formatter, "OIDC discovery failed: {error}"),
            Self::KeySet(error) => write!(formatter, "OIDC key refresh failed: {error}"),
            Self::InvalidToken(error) => write!(formatter, "access token is invalid: {error}"),
            Self::InvalidClaims(error) => {
                write!(formatter, "access token claims are invalid: {error}")
            }
            Self::MissingRole(role) => write!(formatter, "required role {role:?} is missing"),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

impl OidcVerifier {
    pub async fn discover(config: OidcConfig) -> Result<Self, AuthError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .https_only(true)
            .build()
            .map_err(|error| AuthError::Discovery(error.to_string()))?;
        let discovery_url = format!("{}/.well-known/openid-configuration", config.issuer);
        let document =
            fetch_bounded_json::<DiscoveryDocument>(&client, &discovery_url, MAX_DISCOVERY_BYTES)
                .await
                .map_err(AuthError::Discovery)?;
        if document.issuer.trim_end_matches('/') != config.issuer {
            return Err(AuthError::Discovery(format!(
                "provider issuer {:?} did not match configured issuer {:?}",
                document.issuer, config.issuer
            )));
        }
        let jwks_uri = validated_jwks_uri(&config, &document.jwks_uri)?;
        let keys = fetch_keys(&client, &jwks_uri).await?;
        Ok(Self {
            config,
            client,
            jwks_uri,
            keys: Arc::new(RwLock::new(keys)),
            last_jwks_refresh: Arc::new(Mutex::new(Instant::now())),
        })
    }

    pub fn required_role(&self) -> &str {
        &self.config.required_role
    }

    pub async fn verify_bearer(
        &self,
        authorization: &str,
    ) -> Result<AuthenticatedPrincipal, AuthError> {
        self.verify_bearer_for_role(authorization, &self.config.required_role)
            .await
    }

    pub async fn verify_bearer_for_role(
        &self,
        authorization: &str,
        required_role: &str,
    ) -> Result<AuthenticatedPrincipal, AuthError> {
        let token = bearer_token(authorization)?;
        let header =
            decode_header(token).map_err(|error| AuthError::InvalidToken(error.to_string()))?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::InvalidToken(
                "only RS256 access tokens are accepted".to_string(),
            ));
        }
        if let Some(token_type) = header.typ.as_deref()
            && token_type != "JWT"
            && token_type != "at+jwt"
        {
            return Err(AuthError::InvalidToken(format!(
                "unsupported token type {token_type:?}"
            )));
        }
        let kid = header
            .kid
            .as_deref()
            .ok_or_else(|| AuthError::InvalidToken("token header has no kid".to_string()))?;
        let key = self.decoding_key(kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = TOKEN_CLOCK_SKEW;
        validation.validate_nbf = true;
        validation.set_audience(&[&self.config.audience]);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
        let claims = decode::<Value>(token, &key, &validation)
            .map_err(|error| AuthError::InvalidToken(error.to_string()))?
            .claims;
        principal_from_claims(&self.config, claims, required_role)
    }

    async fn decoding_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(key) = self.find_key(kid)? {
            return Ok(key);
        }

        let mut last_refresh = self.last_jwks_refresh.lock().await;
        if let Some(key) = self.find_key(kid)? {
            return Ok(key);
        }
        if last_refresh.elapsed() < JWKS_REFRESH_COOLDOWN {
            return Err(AuthError::InvalidToken(format!(
                "no signing key matches token kid {kid:?}"
            )));
        }
        // Advance the cooldown before I/O so an unavailable provider cannot be
        // hammered with one JWKS request for every attacker-controlled kid.
        *last_refresh = Instant::now();
        let keys = fetch_keys(&self.client, &self.jwks_uri).await?;
        *self
            .keys
            .write()
            .map_err(|_| AuthError::KeySet("JWKS lock poisoned".to_string()))? = keys;
        self.find_key(kid)?.ok_or_else(|| {
            AuthError::InvalidToken(format!("no signing key matches token kid {kid:?}"))
        })
    }

    fn find_key(&self, kid: &str) -> Result<Option<DecodingKey>, AuthError> {
        let keys = self
            .keys
            .read()
            .map_err(|_| AuthError::KeySet("JWKS lock poisoned".to_string()))?;
        keys.find(kid)
            .map(DecodingKey::from_jwk)
            .transpose()
            .map_err(|error| AuthError::InvalidToken(error.to_string()))
    }
}

fn validated_jwks_uri(config: &OidcConfig, value: &str) -> Result<String, AuthError> {
    let jwks_url = reqwest::Url::parse(value)
        .map_err(|_| AuthError::Discovery("jwks_uri is not a valid URL".to_string()))?;
    if jwks_url.scheme() != "https"
        || jwks_url.host_str().is_none()
        || !jwks_url.username().is_empty()
        || jwks_url.password().is_some()
        || jwks_url.fragment().is_some()
        || jwks_url.origin().ascii_serialization() != config.jwks_origin
    {
        return Err(AuthError::Discovery(
            "jwks_uri must use the configured HTTPS JWKS origin without credentials or a fragment"
                .to_string(),
        ));
    }
    Ok(jwks_url.to_string())
}

async fn fetch_keys(client: &reqwest::Client, uri: &str) -> Result<JwkSet, AuthError> {
    fetch_bounded_json(client, uri, MAX_JWKS_BYTES)
        .await
        .map_err(AuthError::KeySet)
}

async fn fetch_bounded_json<T>(
    client: &reqwest::Client,
    uri: &str,
    max_bytes: usize,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let mut response = client
        .get(uri)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(format!("JSON response exceeds {max_bytes} bytes"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(format!("JSON response exceeds {max_bytes} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

fn bearer_token(authorization: &str) -> Result<&str, AuthError> {
    let mut parts = authorization.split_whitespace();
    let scheme = parts.next().ok_or(AuthError::MissingBearer)?;
    let token = parts.next().ok_or(AuthError::MissingBearer)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() || parts.next().is_some() {
        return Err(AuthError::InvalidBearer);
    }
    Ok(token)
}

fn principal_from_claims(
    config: &OidcConfig,
    claims: Value,
    required_role: &str,
) -> Result<AuthenticatedPrincipal, AuthError> {
    let subject = required_string_claim(&claims, "/sub", "sub", 1_024)?;
    let iat = required_u64_claim(&claims, "/iat", "iat")?;
    let exp = required_u64_claim(&claims, "/exp", "exp")?;
    validate_token_times(
        iat,
        exp,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )?;
    let tenant = required_string_claim(&claims, &config.tenant_pointer, "tenant identity", 128)?;
    let tenant_id = TenantId::from_str(tenant)
        .map_err(|error| AuthError::InvalidClaims(format!("invalid tenant id: {error}")))?;
    let agent_identity = match config.agent_id_pointer.as_deref() {
        Some(pointer) => optional_string_claim(&claims, pointer, "agent identity", 255)?,
        None => match claims
            .pointer("/azp")
            .or_else(|| claims.pointer("/client_id"))
        {
            Some(_) => optional_string_claim(
                &claims,
                if claims.pointer("/azp").is_some() {
                    "/azp"
                } else {
                    "/client_id"
                },
                "agent identity",
                255,
            )?,
            None => None,
        },
    };
    let roles = roles_from_claims(config, &claims)?;
    if !roles.contains(required_role) {
        return Err(AuthError::MissingRole(required_role.to_string()));
    }
    let preferred_username =
        optional_string_claim(&claims, "/preferred_username", "preferred_username", 1_024)?;
    Ok(AuthenticatedPrincipal {
        subject: subject.to_string(),
        agent_identity,
        tenant_id,
        roles,
        preferred_username,
        authorized_until_unix: exp,
    })
}

fn roles_from_claims(config: &OidcConfig, claims: &Value) -> Result<BTreeSet<String>, AuthError> {
    let value = match &config.role_claims {
        OidcRoleClaims::KeycloakResourceAccess => claims
            .get("resource_access")
            .and_then(Value::as_object)
            .and_then(|resources| resources.get(&config.audience))
            .and_then(|resource| resource.get("roles")),
        OidcRoleClaims::ClaimPointer(pointer) => claims.pointer(pointer),
    };
    let Some(value) = value else {
        return Ok(BTreeSet::new());
    };
    let values = match value {
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    AuthError::InvalidClaims("role arrays must contain only strings".to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Value::String(value) => value.split_ascii_whitespace().collect(),
        _ => {
            return Err(AuthError::InvalidClaims(
                "role claim must be a string or an array of strings".to_string(),
            ));
        }
    };
    if values.len() > 256 {
        return Err(AuthError::InvalidClaims(
            "role claim contains more than 256 roles".to_string(),
        ));
    }
    values
        .into_iter()
        .map(|role| {
            let role = role.trim();
            if role.is_empty() || role.len() > 256 || role.chars().any(char::is_control) {
                Err(AuthError::InvalidClaims(
                    "roles must be non-empty strings of at most 256 bytes".to_string(),
                ))
            } else {
                Ok(role.to_string())
            }
        })
        .collect()
}

fn required_string_claim<'a>(
    claims: &'a Value,
    pointer: &str,
    label: &str,
    max_bytes: usize,
) -> Result<&'a str, AuthError> {
    let value = claims
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= max_bytes
                && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            AuthError::InvalidClaims(format!(
                "{label} claim at {pointer:?} must be a non-empty string of at most {max_bytes} bytes"
            ))
        })?;
    Ok(value)
}

fn optional_string_claim(
    claims: &Value,
    pointer: &str,
    label: &str,
    max_bytes: usize,
) -> Result<Option<String>, AuthError> {
    match claims.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => required_string_claim(claims, pointer, label, max_bytes)
            .map(|value| Some(value.to_string())),
    }
}

fn required_u64_claim(claims: &Value, pointer: &str, label: &str) -> Result<u64, AuthError> {
    claims
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            AuthError::InvalidClaims(format!(
                "{label} claim at {pointer:?} must be a non-negative integer"
            ))
        })
}

fn validate_token_times(iat: u64, exp: u64, now: u64) -> Result<(), AuthError> {
    if exp > MAX_AUTHORIZATION_UNIX
        || exp <= iat
        || exp.saturating_sub(iat) > MAX_ACCESS_TOKEN_LIFETIME
        || iat > now.saturating_add(TOKEN_CLOCK_SKEW)
        || exp > now.saturating_add(MAX_ACCESS_TOKEN_LIFETIME + TOKEN_CLOCK_SKEW)
    {
        return Err(AuthError::InvalidClaims(
            "access token issue/expiry timestamps are outside the accepted range".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> OidcConfig {
        OidcConfig {
            issuer: "https://id.example/realms/skid".to_string(),
            jwks_origin: "https://id.example".to_string(),
            audience: "skid-client-api".to_string(),
            required_role: "telemetry-read".to_string(),
            tenant_pointer: "/tenant_id".to_string(),
            role_claims: OidcRoleClaims::KeycloakResourceAccess,
            agent_id_pointer: None,
        }
    }

    #[test]
    fn bearer_parser_rejects_extra_or_wrong_scheme() {
        assert_eq!(bearer_token("Bearer abc").unwrap(), "abc");
        assert!(matches!(
            bearer_token("Basic abc"),
            Err(AuthError::InvalidBearer)
        ));
        assert!(matches!(
            bearer_token("Bearer abc extra"),
            Err(AuthError::InvalidBearer)
        ));
    }

    #[test]
    fn jwks_can_use_another_path_on_only_the_configured_origin() {
        let config = config();
        assert_eq!(
            validated_jwks_uri(&config, "https://id.example/tenant/discovery/v2.0/keys").unwrap(),
            "https://id.example/tenant/discovery/v2.0/keys"
        );
        assert!(validated_jwks_uri(&config, "https://keys.example/jwks").is_err());
        assert!(validated_jwks_uri(&config, "http://id.example/jwks").is_err());
        assert!(validated_jwks_uri(&config, "https://user@id.example/jwks").is_err());
    }

    #[test]
    fn principal_requires_audience_scoped_role_and_tenant() {
        let tenant = uuid::Uuid::new_v4();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = json!({
            "sub": "user-1",
            "iat": now,
            "exp": now + 300,
            "azp": "skid-web",
            "preferred_username": "operator",
            "tenant_id": tenant.to_string(),
            "resource_access": {
                "skid-client-api": { "roles": ["telemetry-read"] }
            }
        });
        let principal = principal_from_claims(&config(), claims, "telemetry-read").unwrap();

        assert_eq!(principal.tenant_id.as_uuid(), tenant);
        assert_eq!(principal.agent_identity.as_deref(), Some("skid-web"));
        assert!(principal.has_role("telemetry-read"));
    }

    #[test]
    fn role_from_another_audience_is_not_accepted() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = json!({
            "sub": "user-1",
            "iat": now,
            "exp": now + 300,
            "azp": "skid-web",
            "tenant_id": uuid::Uuid::new_v4().to_string(),
            "resource_access": {
                "other-api": { "roles": ["telemetry-read"] }
            }
        });

        assert!(matches!(
            principal_from_claims(&config(), claims, "telemetry-read"),
            Err(AuthError::MissingRole(_))
        ));
    }

    #[test]
    fn generic_claim_profile_supports_nested_tenant_roles_and_agent_identity() {
        let tenant = uuid::Uuid::new_v4();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let config = OidcConfig {
            issuer: "https://accounts.example".to_string(),
            jwks_origin: "https://accounts.example".to_string(),
            audience: "skid-client-api".to_string(),
            required_role: "telemetry-read".to_string(),
            tenant_pointer: "/organization/id".to_string(),
            role_claims: OidcRoleClaims::ClaimPointer(
                "/https:~1~1monitor.example~1roles".to_string(),
            ),
            agent_id_pointer: Some("/appid".to_string()),
        };
        let claims = json!({
            "sub": "user-1",
            "iat": now,
            "exp": now + 300,
            "appid": "agent-from-provider",
            "organization": { "id": tenant.to_string() },
            "https://monitor.example/roles": ["telemetry-read", "telemetry-admin"]
        });

        let principal = principal_from_claims(&config, claims, "telemetry-read").unwrap();
        assert_eq!(principal.tenant_id.as_uuid(), tenant);
        assert_eq!(
            principal.agent_identity.as_deref(),
            Some("agent-from-provider")
        );
        assert!(principal.has_role("telemetry-admin"));
    }

    #[test]
    fn generic_string_role_claim_accepts_oauth_scope_shape() {
        let mut config = config();
        config.role_claims = OidcRoleClaims::ClaimPointer("/scope".to_string());
        let claims = json!({
            "sub": "user-1",
            "iat": 2_000_000_000_u64,
            "exp": 2_000_000_300_u64,
            "client_id": "agent-1",
            "tenant_id": uuid::Uuid::new_v4().to_string(),
            "scope": "openid telemetry-read profile"
        });

        assert!(
            roles_from_claims(&config, &claims)
                .unwrap()
                .contains("telemetry-read")
        );
    }

    #[test]
    fn user_tokens_do_not_require_an_agent_identity_claim() {
        let tenant = uuid::Uuid::new_v4();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = json!({
            "sub": "user-without-client-id",
            "iat": now,
            "exp": now + 300,
            "tenant_id": tenant.to_string(),
            "resource_access": {
                "skid-client-api": { "roles": ["telemetry-read"] }
            }
        });

        let principal = principal_from_claims(&config(), claims, "telemetry-read").unwrap();
        assert!(principal.agent_identity.is_none());
        assert!(principal.agent_id().is_err());
    }

    #[test]
    fn access_token_lifetime_is_bounded() {
        let now = 2_000_000_000;
        assert!(validate_token_times(now, now + 300, now).is_ok());
        assert!(validate_token_times(now, now, now).is_err());
        assert!(validate_token_times(now, now + MAX_ACCESS_TOKEN_LIFETIME + 1, now).is_err());
        assert!(validate_token_times(now + TOKEN_CLOCK_SKEW + 1, now + 300, now).is_err());
    }
}
