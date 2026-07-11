use crate::config::JwtConfig;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use skid_monitor_core::{AgentId, TenantId};
use std::collections::{BTreeSet, HashMap};
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
    pub client_id: String,
    pub tenant_id: TenantId,
    pub roles: BTreeSet<String>,
    pub preferred_username: Option<String>,
    pub authorized_until_unix: u64,
}

impl AuthenticatedPrincipal {
    pub fn agent_id(&self) -> Result<AgentId, AuthError> {
        AgentId::new(&self.client_id).map_err(|error| AuthError::InvalidClaims(error.to_string()))
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(role)
    }
}

#[derive(Clone)]
pub struct JwtVerifier {
    config: JwtConfig,
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

#[derive(Clone, Debug, Deserialize)]
struct AccessClaims {
    sub: String,
    iat: u64,
    exp: u64,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    resource_access: HashMap<String, ClientRoleClaims>,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ClientRoleClaims {
    #[serde(default)]
    roles: Vec<String>,
}

impl JwtVerifier {
    pub async fn discover(config: JwtConfig) -> Result<Self, AuthError> {
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
        if !document
            .jwks_uri
            .starts_with(&format!("{}/", config.issuer))
        {
            return Err(AuthError::Discovery(
                "jwks_uri must remain below the configured issuer URL".to_string(),
            ));
        }
        let keys = fetch_keys(&client, &document.jwks_uri).await?;
        Ok(Self {
            config,
            client,
            jwks_uri: document.jwks_uri,
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
        let claims = decode::<AccessClaims>(token, &key, &validation)
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
    config: &JwtConfig,
    claims: AccessClaims,
    required_role: &str,
) -> Result<AuthenticatedPrincipal, AuthError> {
    let subject = claims.sub.trim();
    if subject.is_empty() || subject.len() > 1_024 || subject.chars().any(char::is_control) {
        return Err(AuthError::InvalidClaims(
            "sub must be a non-empty identifier of at most 1024 bytes".to_string(),
        ));
    }
    validate_token_times(
        claims.iat,
        claims.exp,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )?;
    let tenant = claims
        .extra
        .get(&config.tenant_claim)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AuthError::InvalidClaims(format!(
                "required tenant claim {:?} is missing or is not a string",
                config.tenant_claim
            ))
        })?;
    let tenant_id = TenantId::from_str(tenant)
        .map_err(|error| AuthError::InvalidClaims(format!("invalid tenant id: {error}")))?;
    let client_id = claims
        .azp
        .or(claims.client_id)
        .ok_or_else(|| AuthError::InvalidClaims("azp/client_id is missing".to_string()))?;
    let client_id = client_id.trim();
    if client_id.is_empty() || client_id.len() > 255 || client_id.chars().any(char::is_control) {
        return Err(AuthError::InvalidClaims(
            "azp/client_id must be a non-empty identifier of at most 255 bytes".to_string(),
        ));
    }
    let roles = claims
        .resource_access
        .get(&config.audience)
        .map(|access| access.roles.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    if !roles.contains(required_role) {
        return Err(AuthError::MissingRole(required_role.to_string()));
    }
    Ok(AuthenticatedPrincipal {
        subject: subject.to_string(),
        client_id: client_id.to_string(),
        tenant_id,
        roles,
        preferred_username: claims.preferred_username,
        authorized_until_unix: claims.exp,
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

    fn config() -> JwtConfig {
        JwtConfig {
            issuer: "https://id.example/realms/skid".to_string(),
            audience: "skid-client-api".to_string(),
            required_role: "telemetry-read".to_string(),
            tenant_claim: "tenant_id".to_string(),
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
    fn principal_requires_audience_scoped_role_and_tenant() {
        let tenant = uuid::Uuid::new_v4();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims: AccessClaims = serde_json::from_value(json!({
            "sub": "user-1",
            "iat": now,
            "exp": now + 300,
            "azp": "skid-web",
            "preferred_username": "operator",
            "tenant_id": tenant.to_string(),
            "resource_access": {
                "skid-client-api": { "roles": ["telemetry-read"] }
            }
        }))
        .unwrap();
        let principal = principal_from_claims(&config(), claims, "telemetry-read").unwrap();

        assert_eq!(principal.tenant_id.as_uuid(), tenant);
        assert_eq!(principal.client_id, "skid-web");
        assert!(principal.has_role("telemetry-read"));
    }

    #[test]
    fn role_from_another_audience_is_not_accepted() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims: AccessClaims = serde_json::from_value(json!({
            "sub": "user-1",
            "iat": now,
            "exp": now + 300,
            "azp": "skid-web",
            "tenant_id": uuid::Uuid::new_v4().to_string(),
            "resource_access": {
                "other-api": { "roles": ["telemetry-read"] }
            }
        }))
        .unwrap();

        assert!(matches!(
            principal_from_claims(&config(), claims, "telemetry-read"),
            Err(AuthError::MissingRole(_))
        ));
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
