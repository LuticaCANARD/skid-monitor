use serde::{Deserialize, Serialize};
use skid_protocol::protocol::Signal;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use uuid::Uuid;
use web_time::{SystemTime, UNIX_EPOCH};

const MAX_AGENT_ID_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for EventId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl FromStr for EventId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Display for TenantId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl FromStr for TenantId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(IdError::Empty);
        }
        if trimmed.len() > MAX_AGENT_ID_BYTES {
            return Err(IdError::TooLong {
                max: MAX_AGENT_ID_BYTES,
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(IdError::ControlCharacter);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AgentId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for AgentId {
    type Err = IdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdError {
    Empty,
    TooLong { max: usize },
    ControlCharacter,
}

impl Display for IdError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier must not be empty"),
            Self::TooLong { max } => write!(formatter, "identifier must be at most {max} bytes"),
            Self::ControlCharacter => {
                formatter.write_str("identifier must not contain control characters")
            }
        }
    }
}

impl std::error::Error for IdError {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SignalScope {
    Solo,
    Tenant { tenant_id: TenantId },
}

impl SignalScope {
    pub const fn tenant(tenant_id: TenantId) -> Self {
        Self::Tenant { tenant_id }
    }

    pub const fn tenant_id(self) -> Option<TenantId> {
        match self {
            Self::Solo => None,
            Self::Tenant { tenant_id } => Some(tenant_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Metrics,
    Traces,
    Logs,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SignalEnvelope {
    pub event_id: EventId,
    pub scope: SignalScope,
    pub agent_id: AgentId,
    pub sequence: u64,
    pub received_at_unix_nano: u64,
    pub payload: Signal,
}

impl SignalEnvelope {
    pub fn new(
        scope: SignalScope,
        agent_id: AgentId,
        sequence: u64,
        received_at_unix_nano: u64,
        payload: Signal,
    ) -> Self {
        Self {
            event_id: EventId::new(),
            scope,
            agent_id,
            sequence,
            received_at_unix_nano,
            payload,
        }
    }

    pub fn now(scope: SignalScope, agent_id: AgentId, sequence: u64, payload: Signal) -> Self {
        let received_at_unix_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        Self::new(scope, agent_id, sequence, received_at_unix_nano, payload)
    }

    pub fn kind(&self) -> SignalKind {
        match &self.payload {
            Signal::Metrics(_) => SignalKind::Metrics,
            Signal::Traces(_) => SignalKind::Traces,
            Signal::Logs(_) => SignalKind::Logs,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SignalCursor(pub u64);

#[cfg(test)]
mod tests {
    use super::*;
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
    fn envelope_round_trips_without_changing_signal_contract() {
        let envelope = SignalEnvelope::new(
            SignalScope::Solo,
            AgentId::new("local-agent").unwrap(),
            7,
            42,
            signal(),
        );
        let json = serde_json::to_string(&envelope).unwrap();
        let decoded: SignalEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.scope, SignalScope::Solo);
        assert_eq!(decoded.agent_id.as_str(), "local-agent");
        assert_eq!(decoded.sequence, 7);
        assert_eq!(decoded.kind(), SignalKind::Metrics);
    }

    #[test]
    fn agent_ids_reject_ambiguous_input() {
        assert_eq!(AgentId::new("  ").unwrap_err(), IdError::Empty);
        assert_eq!(
            AgentId::new("agent\nother").unwrap_err(),
            IdError::ControlCharacter
        );
    }
}
