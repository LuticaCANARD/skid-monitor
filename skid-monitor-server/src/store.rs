use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use skid_monitor_core::{
    AgentId, AppendOutcome, EventId, SignalCursor, SignalEnvelope, SignalProjection, SignalReader,
    SignalRecord, SignalScope, SignalWriter, StoreFuture, TenantId,
};
use skid_protocol::protocol::Signal;
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::io::{self, Write};
use std::time::Duration;
use uuid::Uuid;

pub const SIGNAL_NOTIFY_CHANNEL: &str = "skid_monitor_signal_events";
pub const MAX_LOAD_LIMIT: usize = 1_000;
pub const MAX_REPLAY_JSON_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_STORED_SIGNAL_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_AGENTS_PER_TENANT: usize = 1_000;
pub const MAX_STREAM_TICKET_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_STREAM_TICKET_SUBJECT_BYTES: usize = 1_024;
const SET_SAFE_SEARCH_PATH_SQL: &str =
    "SELECT pg_catalog.set_config('search_path', 'pg_catalog, public', false)";
// Keep authorization timestamps inside the portable RFC 3339 year-9999
// domain even though PostgreSQL itself supports a wider timestamp range.
const MAX_STREAM_TICKET_AUTHORIZATION_UNIX: u64 = 253_402_300_799;

static EMBEDDED_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");

const APPLIED_MIGRATIONS_SQL: &str =
    "SELECT version, success, checksum FROM public._sqlx_migrations ORDER BY version";

const RUNTIME_RELATIONS_SQL: &str = r#"
SELECT
    relations.relname AS relation_name,
    relations.relrowsecurity AS row_security,
    relations.relforcerowsecurity AS force_row_security
FROM pg_catalog.pg_class AS relations
JOIN pg_catalog.pg_namespace AS namespaces ON namespaces.oid = relations.relnamespace
WHERE namespaces.nspname = 'public'
  AND relations.relkind IN ('r', 'p')
  AND relations.relname = ANY($1::text[])
"#;

const RUNTIME_COLUMNS_SQL: &str = r#"
SELECT
    relations.relname AS relation_name,
    attributes.attname AS column_name,
    pg_catalog.format_type(attributes.atttypid, attributes.atttypmod) AS data_type,
    attributes.attnotnull AS not_null,
    attributes.attidentity::text AS identity
FROM pg_catalog.pg_attribute AS attributes
JOIN pg_catalog.pg_class AS relations ON relations.oid = attributes.attrelid
JOIN pg_catalog.pg_namespace AS namespaces ON namespaces.oid = relations.relnamespace
WHERE namespaces.nspname = 'public'
  AND relations.relkind IN ('r', 'p')
  AND attributes.attnum > 0
  AND NOT attributes.attisdropped
  AND relations.relname = ANY($1::text[])
"#;

const RUNTIME_CONSTRAINTS_SQL: &str = r#"
SELECT
    relations.relname AS relation_name,
    constraints.conname AS constraint_name,
    constraints.contype::text AS constraint_type,
    constraints.convalidated AS validated
FROM pg_catalog.pg_constraint AS constraints
JOIN pg_catalog.pg_class AS relations ON relations.oid = constraints.conrelid
JOIN pg_catalog.pg_namespace AS namespaces ON namespaces.oid = relations.relnamespace
WHERE namespaces.nspname = 'public'
  AND relations.relname = ANY($1::text[])
"#;

const RUNTIME_INDEXES_SQL: &str = r#"
SELECT
    relations.relname AS relation_name,
    indexes.relname AS index_name,
    catalog_indexes.indisvalid AS valid,
    catalog_indexes.indisready AS ready
FROM pg_catalog.pg_index AS catalog_indexes
JOIN pg_catalog.pg_class AS relations ON relations.oid = catalog_indexes.indrelid
JOIN pg_catalog.pg_class AS indexes ON indexes.oid = catalog_indexes.indexrelid
JOIN pg_catalog.pg_namespace AS namespaces ON namespaces.oid = relations.relnamespace
WHERE namespaces.nspname = 'public'
  AND relations.relname = ANY($1::text[])
"#;

const RUNTIME_POLICIES_SQL: &str = r#"
SELECT
    relations.relname AS relation_name,
    policies.polname AS policy_name,
    policies.polpermissive AS permissive,
    policies.polcmd::text AS command,
    policies.polroles = ARRAY[0::oid] AS public_only,
    pg_catalog.pg_get_expr(policies.polqual, policies.polrelid) AS using_expression,
    pg_catalog.pg_get_expr(policies.polwithcheck, policies.polrelid) AS check_expression
FROM pg_catalog.pg_policy AS policies
JOIN pg_catalog.pg_class AS relations ON relations.oid = policies.polrelid
JOIN pg_catalog.pg_namespace AS namespaces ON namespaces.oid = relations.relnamespace
WHERE namespaces.nspname = 'public'
  AND relations.relname = ANY($1::text[])
"#;

const MIGRATION_SELECT_PRIVILEGE_SQL: &str =
    "SELECT has_table_privilege(current_user, 'public._sqlx_migrations', 'SELECT')";

const AGENT_CARDINALITY_SQL: &str = "SELECT count(*)::bigint AS agent_count, \
     count(*) FILTER (WHERE agent_id = $2) > 0 AS already_enrolled \
     FROM agents WHERE tenant_id = $1";

const TENANT_RELATIONS: &[&str] = &[
    "tenants",
    "agents",
    "signal_events",
    "signal_projection",
    "audit_events",
    "stream_tickets",
];

const REQUIRED_RELATIONS: &[&str] = &[
    "_sqlx_migrations",
    "tenants",
    "agents",
    "signal_events",
    "signal_projection",
    "audit_events",
    "stream_tickets",
];

#[derive(Clone, Copy)]
struct RequiredColumn {
    relation: &'static str,
    name: &'static str,
    data_type: &'static str,
    not_null: bool,
    identity: &'static str,
}

macro_rules! column {
    ($relation:literal, $name:literal, $data_type:literal, $not_null:literal) => {
        RequiredColumn {
            relation: $relation,
            name: $name,
            data_type: $data_type,
            not_null: $not_null,
            identity: "",
        }
    };
    ($relation:literal, $name:literal, $data_type:literal, $not_null:literal, $identity:literal) => {
        RequiredColumn {
            relation: $relation,
            name: $name,
            data_type: $data_type,
            not_null: $not_null,
            identity: $identity,
        }
    };
}

const REQUIRED_COLUMNS: &[RequiredColumn] = &[
    column!("_sqlx_migrations", "version", "bigint", true),
    column!("_sqlx_migrations", "description", "text", true),
    column!(
        "_sqlx_migrations",
        "installed_on",
        "timestamp with time zone",
        true
    ),
    column!("_sqlx_migrations", "success", "boolean", true),
    column!("_sqlx_migrations", "checksum", "bytea", true),
    column!("_sqlx_migrations", "execution_time", "bigint", true),
    column!("tenants", "id", "uuid", true),
    column!("tenants", "slug", "text", true),
    column!("tenants", "display_name", "text", true),
    column!("tenants", "enabled", "boolean", true),
    column!("tenants", "created_at", "timestamp with time zone", true),
    column!("tenants", "updated_at", "timestamp with time zone", true),
    column!("agents", "tenant_id", "uuid", true),
    column!("agents", "agent_id", "text", true),
    column!("agents", "display_name", "text", false),
    column!("agents", "enabled", "boolean", true),
    column!("agents", "enrolled_at", "timestamp with time zone", true),
    column!("agents", "updated_at", "timestamp with time zone", true),
    column!("agents", "last_seen_at", "timestamp with time zone", false),
    column!("signal_events", "cursor", "bigint", true, "a"),
    column!("signal_events", "tenant_id", "uuid", true),
    column!("signal_events", "event_id", "uuid", true),
    column!("signal_events", "agent_id", "text", true),
    column!("signal_events", "sequence", "numeric(20,0)", true),
    column!(
        "signal_events",
        "received_at_unix_nano",
        "numeric(20,0)",
        true
    ),
    column!("signal_events", "signal_kind", "text", true),
    column!("signal_events", "payload", "jsonb", true),
    column!("signal_events", "payload_bytes", "bigint", true),
    column!(
        "signal_events",
        "committed_at",
        "timestamp with time zone",
        true
    ),
    column!("signal_projection", "tenant_id", "uuid", true),
    column!("signal_projection", "last_cursor", "bigint", true),
    column!("signal_projection", "projection", "jsonb", true),
    column!(
        "signal_projection",
        "updated_at",
        "timestamp with time zone",
        true
    ),
    column!("audit_events", "id", "bigint", true, "a"),
    column!("audit_events", "tenant_id", "uuid", true),
    column!("audit_events", "actor_type", "text", true),
    column!("audit_events", "actor_id", "text", true),
    column!("audit_events", "action", "text", true),
    column!("audit_events", "target_type", "text", true),
    column!("audit_events", "target_id", "text", true),
    column!("audit_events", "details", "jsonb", true),
    column!(
        "audit_events",
        "occurred_at",
        "timestamp with time zone",
        true
    ),
    column!("stream_tickets", "ticket_id", "uuid", true),
    column!("stream_tickets", "tenant_id", "uuid", true),
    column!("stream_tickets", "subject", "text", true),
    column!(
        "stream_tickets",
        "created_at",
        "timestamp with time zone",
        true
    ),
    column!(
        "stream_tickets",
        "authorized_until",
        "timestamp with time zone",
        true
    ),
    column!(
        "stream_tickets",
        "expires_at",
        "timestamp with time zone",
        true
    ),
    column!(
        "stream_tickets",
        "consumed_at",
        "timestamp with time zone",
        false
    ),
];

#[derive(Clone, Copy)]
struct RequiredConstraint {
    relation: &'static str,
    name: &'static str,
    constraint_type: &'static str,
}

macro_rules! constraint {
    ($relation:literal, $name:literal, $constraint_type:literal) => {
        RequiredConstraint {
            relation: $relation,
            name: $name,
            constraint_type: $constraint_type,
        }
    };
}

const REQUIRED_CONSTRAINTS: &[RequiredConstraint] = &[
    constraint!("_sqlx_migrations", "_sqlx_migrations_pkey", "p"),
    constraint!("tenants", "tenants_pkey", "p"),
    constraint!("tenants", "tenants_slug_key", "u"),
    constraint!("tenants", "tenants_slug_not_blank", "c"),
    constraint!("tenants", "tenants_display_name_not_blank", "c"),
    constraint!("agents", "agents_pkey", "p"),
    constraint!("agents", "agents_tenant_id_fkey", "f"),
    constraint!("agents", "agents_id_size", "c"),
    constraint!("agents", "agents_id_no_control_characters", "c"),
    constraint!("agents", "agents_display_name_not_blank", "c"),
    constraint!("signal_events", "signal_events_pkey", "p"),
    constraint!("signal_events", "signal_events_tenant_id_fkey", "f"),
    constraint!("signal_events", "signal_events_agent_fk", "f"),
    constraint!("signal_events", "signal_events_tenant_event_unique", "u"),
    constraint!(
        "signal_events",
        "signal_events_tenant_agent_sequence_unique",
        "u"
    ),
    constraint!("signal_events", "signal_events_cursor_positive", "c"),
    constraint!("signal_events", "signal_events_sequence_non_negative", "c"),
    constraint!("signal_events", "signal_events_sequence_u64", "c"),
    constraint!(
        "signal_events",
        "signal_events_received_at_non_negative",
        "c"
    ),
    constraint!("signal_events", "signal_events_received_at_u64", "c"),
    constraint!("signal_events", "signal_events_kind", "c"),
    constraint!("signal_events", "signal_events_payload_object", "c"),
    constraint!("signal_events", "signal_events_payload_bytes_positive", "c"),
    constraint!("signal_events", "signal_events_payload_bytes_bounded", "c"),
    constraint!("signal_projection", "signal_projection_pkey", "p"),
    constraint!("signal_projection", "signal_projection_tenant_id_fkey", "f"),
    constraint!(
        "signal_projection",
        "signal_projection_last_cursor_non_negative",
        "c"
    ),
    constraint!("signal_projection", "signal_projection_object", "c"),
    constraint!("audit_events", "audit_events_pkey", "p"),
    constraint!("audit_events", "audit_events_tenant_id_fkey", "f"),
    constraint!("audit_events", "audit_events_actor_type_not_blank", "c"),
    constraint!("audit_events", "audit_events_actor_id_not_blank", "c"),
    constraint!("audit_events", "audit_events_action_not_blank", "c"),
    constraint!("audit_events", "audit_events_target_type_not_blank", "c"),
    constraint!("audit_events", "audit_events_target_id_not_blank", "c"),
    constraint!("audit_events", "audit_events_details_object", "c"),
    constraint!("stream_tickets", "stream_tickets_pkey", "p"),
    constraint!("stream_tickets", "stream_tickets_tenant_id_fkey", "f"),
    constraint!("stream_tickets", "stream_tickets_subject_not_blank", "c"),
    constraint!("stream_tickets", "stream_tickets_subject_size", "c"),
    constraint!(
        "stream_tickets",
        "stream_tickets_authorization_after_creation",
        "c"
    ),
    constraint!(
        "stream_tickets",
        "stream_tickets_expiry_after_creation",
        "c"
    ),
    constraint!(
        "stream_tickets",
        "stream_tickets_expiry_within_authorization",
        "c"
    ),
    constraint!(
        "stream_tickets",
        "stream_tickets_consumed_before_expiry",
        "c"
    ),
];

const REQUIRED_INDEXES: &[(&str, &str)] = &[
    ("signal_events", "signal_events_tenant_cursor_idx"),
    ("signal_events", "signal_events_tenant_committed_at_idx"),
    ("audit_events", "audit_events_tenant_occurred_at_idx"),
    ("stream_tickets", "stream_tickets_cleanup_idx"),
];

const CREATE_STREAM_TICKET_SQL: &str = "INSERT INTO stream_tickets (\
         ticket_id, tenant_id, subject, authorized_until, expires_at\
     ) SELECT $1, $2, $3, authorization.authorized_until, \
              LEAST(\
                  statement_timestamp() + ($4 * INTERVAL '1 millisecond'),\
                  authorization.authorized_until\
              ) \
       FROM (\
           SELECT to_timestamp($5::double precision) AS authorized_until\
       ) AS authorization \
       WHERE authorization.authorized_until > statement_timestamp()";

const CONSUME_STREAM_TICKET_SQL: &str = "UPDATE stream_tickets SET consumed_at = statement_timestamp() \
     WHERE tenant_id = $1 AND ticket_id = $2 \
       AND consumed_at IS NULL AND expires_at > statement_timestamp() \
     RETURNING subject, \
         (EXTRACT(EPOCH FROM authorized_until))::bigint AS authorized_until_unix";

const REQUIRE_ENABLED_TENANT_SQL: &str = "SELECT enabled FROM tenants WHERE id = $1 FOR SHARE";

const VERIFY_RUNTIME_ROLE_SQL: &str = r#"
SELECT
    current_user::text AS role_name,
    session_user <> current_user AS session_user_differs,
    roles.rolsuper,
    roles.rolbypassrls,
    roles.rolcreatedb,
    roles.rolcreaterole,
    roles.rolreplication,
    pg_catalog.has_schema_privilege(current_user, 'public', 'CREATE')
        AS can_create_public_schema,
    EXISTS (
        SELECT 1
        FROM pg_catalog.pg_class AS relations
        JOIN pg_catalog.pg_namespace AS namespaces
          ON namespaces.oid = relations.relnamespace
        WHERE namespaces.nspname = 'public'
          AND relations.relkind IN ('r', 'p')
          AND relations.relname = ANY($1::text[])
          AND pg_catalog.pg_has_role(relations.relowner, 'MEMBER')
    ) AS member_of_relation_owner,
    EXISTS (
        SELECT 1
        FROM pg_catalog.pg_roles AS privileged_roles
        WHERE (
            privileged_roles.rolsuper
            OR privileged_roles.rolbypassrls
            OR privileged_roles.rolcreatedb
            OR privileged_roles.rolcreaterole
            OR privileged_roles.rolreplication
        )
          AND pg_catalog.pg_has_role(privileged_roles.oid, 'MEMBER')
    ) AS member_of_privileged_role
FROM pg_catalog.pg_roles AS roles
WHERE roles.rolname = current_user
"#;

const LOAD_ENVELOPES_BOUNDED_SQL: &str = r#"
WITH candidate_metadata AS MATERIALIZED (
    SELECT cursor, payload_bytes
    FROM signal_events
    WHERE tenant_id = $1 AND cursor > $2
    ORDER BY cursor ASC
    LIMIT $3
),
ranked_metadata AS MATERIALIZED (
    SELECT
        cursor,
        row_number() OVER (ORDER BY cursor ASC) AS ordinal,
        sum(payload_bytes) OVER (
            ORDER BY cursor ASC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
        ) AS cumulative_bytes
    FROM candidate_metadata
),
cutoff AS (
    SELECT max(cursor) AS cursor
    FROM ranked_metadata
    WHERE ordinal = 1 OR cumulative_bytes <= $4
)
SELECT
    events.cursor,
    events.event_id,
    events.agent_id,
    events.sequence::text AS sequence,
    events.received_at_unix_nano::text AS received_at_unix_nano,
    events.payload
FROM signal_events AS events
JOIN cutoff ON events.cursor <= cutoff.cursor
WHERE events.tenant_id = $1 AND events.cursor > $2
ORDER BY events.cursor ASC
"#;

#[derive(Clone, Copy, Debug)]
pub struct PgStoreOptions {
    pub min_connections: u32,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

impl Default for PgStoreOptions {
    fn default() -> Self {
        Self {
            min_connections: 1,
            max_connections: 16,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(10 * 60),
            max_lifetime: Duration::from_secs(30 * 60),
        }
    }
}

impl PgStoreOptions {
    fn validate(self) -> Result<Self, PgStoreError> {
        if self.max_connections == 0 {
            return Err(PgStoreError::InvalidPoolOptions(
                "max_connections must be greater than zero",
            ));
        }
        if self.min_connections > self.max_connections {
            return Err(PgStoreError::InvalidPoolOptions(
                "min_connections must not exceed max_connections",
            ));
        }
        if self.acquire_timeout.is_zero() {
            return Err(PgStoreError::InvalidPoolOptions(
                "acquire_timeout must be greater than zero",
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentRecord {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub display_name: Option<String>,
    pub enabled: bool,
    pub enrolled_at_unix_nano: u64,
    pub last_seen_at_unix_nano: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TenantRecord {
    pub tenant_id: TenantId,
    pub slug: String,
    pub display_name: String,
    pub enabled: bool,
    pub created_at_unix_nano: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionRecord {
    pub tenant_id: TenantId,
    pub last_cursor: SignalCursor,
    pub projection: SignalProjection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamTicketGrant {
    pub subject: String,
    pub authorized_until_unix: u64,
}

#[derive(Debug)]
struct RuntimeRoleCapabilities {
    role_name: String,
    session_user_differs: bool,
    superuser: bool,
    bypasses_rls: bool,
    creates_database: bool,
    creates_role: bool,
    replication: bool,
    can_create_public_schema: bool,
    member_of_relation_owner: bool,
    member_of_privileged_role: bool,
}

#[derive(Debug)]
pub enum PgStoreError {
    Database(sqlx::Error),
    Migration(MigrateError),
    Json(serde_json::Error),
    InvalidSignalPayload(serde_json::Error),
    SignalPayloadNotRoundTripSafe,
    SignalPayloadTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    SoloScope,
    TenantDisabled {
        tenant_id: TenantId,
    },
    AgentNotEnrolled {
        agent_id: AgentId,
    },
    AgentDisabled {
        agent_id: AgentId,
    },
    AgentLimitReached {
        tenant_id: TenantId,
        max_agents: usize,
    },
    SequenceConflict {
        agent_id: AgentId,
        sequence: u64,
    },
    IntegerOutOfRange {
        field: &'static str,
    },
    InvalidAgentId(String),
    InvalidActorId,
    InvalidStreamTicketSubject,
    InvalidStreamTicketTtl,
    InvalidStreamTicketAuthorizationExpiry,
    StreamTicketAuthorizationExpired,
    UnsafeRuntimeRole {
        role_name: String,
        reasons: Vec<&'static str>,
    },
    RuntimeSchemaNotReady {
        problems: Vec<String>,
    },
    InvalidPoolOptions(&'static str),
    CorruptRow(&'static str),
}

impl Display for PgStoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "PostgreSQL store error: {error}"),
            Self::Migration(error) => write!(formatter, "PostgreSQL migration error: {error}"),
            Self::Json(error) => write!(formatter, "signal JSON error: {error}"),
            Self::InvalidSignalPayload(error) => {
                write!(formatter, "signal payload is not JSON round-trip safe: {error}")
            }
            Self::SignalPayloadNotRoundTripSafe => formatter.write_str(
                "signal payload changes during JSON round-trip (for example, a non-finite number)",
            ),
            Self::SignalPayloadTooLarge {
                actual_bytes,
                max_bytes,
            } => write!(
                formatter,
                "serialized signal payload is {actual_bytes} bytes; maximum is {max_bytes} bytes",
            ),
            Self::SoloScope => formatter
                .write_str("the PostgreSQL cloud store accepts tenant-scoped envelopes only"),
            Self::TenantDisabled { tenant_id } => {
                write!(formatter, "tenant `{tenant_id}` is disabled")
            }
            Self::AgentNotEnrolled { agent_id } => {
                write!(
                    formatter,
                    "agent `{agent_id}` is not enrolled for this tenant"
                )
            }
            Self::AgentDisabled { agent_id } => {
                write!(formatter, "agent `{agent_id}` is disabled for this tenant")
            }
            Self::AgentLimitReached {
                tenant_id,
                max_agents,
            } => write!(
                formatter,
                "tenant `{tenant_id}` has reached its limit of {max_agents} agents",
            ),
            Self::SequenceConflict { agent_id, sequence } => write!(
                formatter,
                "agent `{agent_id}` reused sequence {sequence} for a different signal",
            ),
            Self::IntegerOutOfRange { field } => {
                write!(formatter, "`{field}` is outside PostgreSQL BIGINT range")
            }
            Self::InvalidAgentId(error) => write!(formatter, "invalid stored agent id: {error}"),
            Self::InvalidActorId => formatter.write_str("audit actor id must not be empty"),
            Self::InvalidStreamTicketSubject => formatter.write_str(
                "stream ticket subject must be non-empty and no larger than 1024 bytes",
            ),
            Self::InvalidStreamTicketTtl => formatter.write_str(
                "stream ticket TTL must be at least one millisecond and no greater than five minutes",
            ),
            Self::InvalidStreamTicketAuthorizationExpiry => formatter.write_str(
                "stream ticket authorization expiry is outside the supported Unix timestamp range",
            ),
            Self::StreamTicketAuthorizationExpired => {
                formatter.write_str("stream ticket authorization has already expired")
            }
            Self::UnsafeRuntimeRole {
                role_name,
                reasons,
            } => write!(
                formatter,
                "PostgreSQL runtime role `{role_name}` is unsafe ({})",
                reasons.join(", "),
            ),
            Self::RuntimeSchemaNotReady { problems } => write!(
                formatter,
                "PostgreSQL runtime schema is not ready: {}",
                problems.join("; "),
            ),
            Self::InvalidPoolOptions(error) => {
                write!(formatter, "invalid PostgreSQL pool options: {error}")
            }
            Self::CorruptRow(field) => {
                write!(formatter, "PostgreSQL row contains invalid `{field}` data")
            }
        }
    }
}

impl std::error::Error for PgStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Migration(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidSignalPayload(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for PgStoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<MigrateError> for PgStoreError {
    fn from(error: MigrateError) -> Self {
        Self::Migration(error)
    }
}

impl From<serde_json::Error> for PgStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone)]
pub struct PgSignalStore {
    pool: PgPool,
}

impl PgSignalStore {
    pub async fn connect(
        database_url: &str,
        options: PgStoreOptions,
    ) -> Result<Self, PgStoreError> {
        let options = options.validate()?;
        let pool = PgPoolOptions::new()
            .min_connections(options.min_connections)
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .idle_timeout(Some(options.idle_timeout))
            .max_lifetime(Some(options.max_lifetime))
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    sqlx::query(SET_SAFE_SEARCH_PATH_SQL)
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<(), PgStoreError> {
        embedded_migrator().run(&self.pool).await?;
        Ok(())
    }

    /// Rejects PostgreSQL roles which can bypass the tenant RLS boundary.
    /// Runtime services should call this after connecting; migration tooling
    /// may intentionally use a separate, more privileged role.
    pub async fn verify_runtime_role(&self) -> Result<(), PgStoreError> {
        let row = sqlx::query(VERIFY_RUNTIME_ROLE_SQL)
            .bind(required_relation_names())
            .fetch_one(&self.pool)
            .await?;
        validate_runtime_role(RuntimeRoleCapabilities {
            role_name: row.try_get("role_name")?,
            session_user_differs: row.try_get("session_user_differs")?,
            superuser: row.try_get("rolsuper")?,
            bypasses_rls: row.try_get("rolbypassrls")?,
            creates_database: row.try_get("rolcreatedb")?,
            creates_role: row.try_get("rolcreaterole")?,
            replication: row.try_get("rolreplication")?,
            can_create_public_schema: row.try_get("can_create_public_schema")?,
            member_of_relation_owner: row.try_get("member_of_relation_owner")?,
            member_of_privileged_role: row.try_get("member_of_privileged_role")?,
        })
    }

    /// Verifies the exact embedded migration set and every security-sensitive
    /// runtime schema invariant in the `public` schema.
    ///
    /// In addition to normal runtime table grants, the runtime role needs
    /// `SELECT` on `public._sqlx_migrations` so the embedded migration checksum
    /// can be compared without giving the runtime role DDL privileges.
    pub async fn verify_runtime_schema(&self) -> Result<(), PgStoreError> {
        let mut problems = runtime_structure_problems(&self.pool).await?;
        if problems.is_empty() {
            problems.extend(runtime_migration_problems(&self.pool).await?);
        }
        schema_ready(problems)
    }

    /// Performs all non-mutating startup checks expected of runtime services.
    pub async fn verify_ready(&self) -> Result<(), PgStoreError> {
        self.verify_runtime_role().await?;
        self.verify_runtime_schema().await
    }

    pub async fn append_envelope(
        &self,
        envelope: &SignalEnvelope,
    ) -> Result<AppendOutcome, PgStoreError> {
        let tenant_id = tenant_scope(envelope.scope)?;
        let sequence = envelope.sequence.to_string();
        let received_at = envelope.received_at_unix_nano.to_string();
        let payload = canonical_signal_payload(&envelope.payload)?;
        let payload_bytes = payload.byte_len as i64;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;

        // Allocate cursors in commit order for each tenant. Without this lock,
        // concurrent transactions could commit cursor N+1 before N and a client
        // advancing to N+1 would permanently skip N.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(tenant_lock_key(tenant_id))
            .execute(&mut *transaction)
            .await?;

        let enabled = sqlx::query(
            "SELECT enabled FROM agents \
             WHERE tenant_id = $1 AND agent_id = $2 FOR SHARE",
        )
        .bind(tenant_id.as_uuid())
        .bind(envelope.agent_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| row.get::<bool, _>("enabled"));
        match enabled {
            None => {
                return Err(PgStoreError::AgentNotEnrolled {
                    agent_id: envelope.agent_id.clone(),
                });
            }
            Some(false) => {
                return Err(PgStoreError::AgentDisabled {
                    agent_id: envelope.agent_id.clone(),
                });
            }
            Some(true) => {}
        }

        let inserted = sqlx::query(
            "INSERT INTO signal_events (\
                 tenant_id, event_id, agent_id, sequence, received_at_unix_nano, signal_kind, \
                 payload, payload_bytes\
             ) VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6, $7, $8) \
             ON CONFLICT (tenant_id, agent_id, sequence) DO NOTHING \
             RETURNING cursor",
        )
        .bind(tenant_id.as_uuid())
        .bind(envelope.event_id.as_uuid())
        .bind(envelope.agent_id.as_str())
        .bind(&sequence)
        .bind(&received_at)
        .bind(signal_kind_name(envelope))
        .bind(Json(payload.value.clone()))
        .bind(payload_bytes)
        .fetch_optional(&mut *transaction)
        .await?;

        let Some(inserted) = inserted else {
            let row = sqlx::query(
                "SELECT cursor, signal_kind, payload FROM signal_events \
                 WHERE tenant_id = $1 AND agent_id = $2 AND sequence = $3::numeric",
            )
            .bind(tenant_id.as_uuid())
            .bind(envelope.agent_id.as_str())
            .bind(&sequence)
            .fetch_one(&mut *transaction)
            .await?;
            let existing_kind: String = row.try_get("signal_kind")?;
            let Json(existing_payload): Json<Value> = row.try_get("payload")?;
            if existing_kind != signal_kind_name(envelope) || existing_payload != payload.value {
                return Err(PgStoreError::SequenceConflict {
                    agent_id: envelope.agent_id.clone(),
                    sequence: envelope.sequence,
                });
            }
            let cursor = cursor_from_row(&row)?;
            transaction.commit().await?;
            return Ok(AppendOutcome {
                cursor,
                inserted: false,
            });
        };

        let cursor = cursor_from_row(&inserted)?;
        let cursor_i64 = to_pg_i64("cursor", cursor.0)?;

        sqlx::query(
            "INSERT INTO signal_projection (tenant_id, last_cursor, projection) \
             VALUES ($1, 0, $2) ON CONFLICT (tenant_id) DO NOTHING",
        )
        .bind(tenant_id.as_uuid())
        .bind(Json(serde_json::to_value(SignalProjection::default())?))
        .execute(&mut *transaction)
        .await?;

        let row =
            sqlx::query("SELECT projection FROM signal_projection WHERE tenant_id = $1 FOR UPDATE")
                .bind(tenant_id.as_uuid())
                .fetch_one(&mut *transaction)
                .await?;
        let Json(projection_json): Json<Value> = row.try_get("projection")?;
        let mut projection: SignalProjection = serde_json::from_value(projection_json)?;
        projection.observe(envelope);
        sqlx::query(
            "UPDATE signal_projection \
             SET last_cursor = GREATEST(last_cursor, $2), projection = $3, updated_at = now() \
             WHERE tenant_id = $1",
        )
        .bind(tenant_id.as_uuid())
        .bind(cursor_i64)
        .bind(Json(serde_json::to_value(&projection)?))
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            "UPDATE agents SET last_seen_at = now(), updated_at = now() \
             WHERE tenant_id = $1 AND agent_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(envelope.agent_id.as_str())
        .execute(&mut *transaction)
        .await?;

        let notification = json!({
            "tenant_id": tenant_id,
            "cursor": cursor.0,
        })
        .to_string();
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(SIGNAL_NOTIFY_CHANNEL)
            .bind(notification)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(AppendOutcome {
            cursor,
            inserted: true,
        })
    }

    pub async fn load_envelopes_after(
        &self,
        scope: SignalScope,
        after: SignalCursor,
        limit: usize,
    ) -> Result<Vec<SignalRecord>, PgStoreError> {
        self.load_envelopes_after_bounded(scope, after, limit, MAX_REPLAY_JSON_BYTES)
            .await
    }

    /// Replays committed events without allowing a large row limit to multiply
    /// the maximum signal payload into an unbounded database response.
    ///
    /// `row_limit` and `max_json_bytes` are both capped by store-wide maxima.
    /// When `row_limit` is non-zero, the first matching event is returned even
    /// when that single payload exceeds the byte budget, so consumers can
    /// always advance their cursor.
    pub async fn load_envelopes_after_bounded(
        &self,
        scope: SignalScope,
        after: SignalCursor,
        row_limit: usize,
        max_json_bytes: usize,
    ) -> Result<Vec<SignalRecord>, PgStoreError> {
        let tenant_id = tenant_scope(scope)?;
        let after = to_pg_i64("cursor", after.0)?;
        let row_limit = bounded_load_limit(row_limit);
        let max_json_bytes = bounded_replay_json_bytes(max_json_bytes);
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;
        let rows = sqlx::query(LOAD_ENVELOPES_BOUNDED_SQL)
            .bind(tenant_id.as_uuid())
            .bind(after)
            .bind(row_limit)
            .bind(max_json_bytes)
            .fetch_all(&mut *transaction)
            .await?;

        let records = rows
            .iter()
            .map(|row| signal_record_from_row(row, tenant_id))
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        Ok(records)
    }

    pub async fn upsert_tenant(
        &self,
        tenant_id: TenantId,
        slug: String,
        display_name: String,
    ) -> Result<TenantRecord, PgStoreError> {
        self.upsert_tenant_as(tenant_id, slug, display_name, "client-api")
            .await
    }

    pub async fn upsert_tenant_as(
        &self,
        tenant_id: TenantId,
        slug: String,
        display_name: String,
        actor_id: &str,
    ) -> Result<TenantRecord, PgStoreError> {
        let actor_id = required_actor_id(actor_id)?;
        let slug = slug.trim();
        let display_name = display_name.trim();
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        let row = sqlx::query(
            "INSERT INTO tenants (id, slug, display_name, enabled) VALUES ($1, $2, $3, TRUE) \
             ON CONFLICT (id) DO UPDATE \
             SET slug = EXCLUDED.slug, display_name = EXCLUDED.display_name, updated_at = now() \
             RETURNING slug, display_name, enabled, \
                 (EXTRACT(EPOCH FROM created_at) * 1000000000)::bigint \
                     AS created_at_unix_nano",
        )
        .bind(tenant_id.as_uuid())
        .bind(slug)
        .bind(display_name)
        .fetch_one(&mut *transaction)
        .await?;
        let tenant = tenant_record_from_row(&row, tenant_id)?;
        if !tenant.enabled {
            return Err(PgStoreError::TenantDisabled { tenant_id });
        }
        sqlx::query(
            "INSERT INTO audit_events (\
                 tenant_id, actor_type, actor_id, action, target_type, target_id, details\
             ) VALUES ($1, 'user', $2, 'tenant.upsert', 'tenant', $3, $4)",
        )
        .bind(tenant_id.as_uuid())
        .bind(actor_id)
        .bind(tenant_id.to_string())
        .bind(Json(json!({
            "slug": slug,
            "display_name": display_name,
        })))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(tenant)
    }

    pub async fn list_agents(&self, tenant_id: TenantId) -> Result<Vec<AgentRecord>, PgStoreError> {
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;
        let rows = sqlx::query(
            "SELECT agent_id, display_name, enabled, \
                    (EXTRACT(EPOCH FROM enrolled_at) * 1000000000)::bigint \
                        AS enrolled_at_unix_nano, \
                    (EXTRACT(EPOCH FROM last_seen_at) * 1000000000)::bigint \
                        AS last_seen_at_unix_nano \
             FROM agents WHERE tenant_id = $1 ORDER BY agent_id ASC",
        )
        .bind(tenant_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await?;
        let agents = rows
            .iter()
            .map(|row| agent_record_from_row(row, tenant_id))
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await?;
        Ok(agents)
    }

    pub async fn load_projection(
        &self,
        tenant_id: TenantId,
    ) -> Result<Option<ProjectionRecord>, PgStoreError> {
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;

        let row = sqlx::query(
            "SELECT last_cursor, projection FROM signal_projection WHERE tenant_id = $1",
        )
        .bind(tenant_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await?;
        let projection = row
            .map(|row| {
                let last_cursor =
                    SignalCursor(from_pg_i64("last_cursor", row.try_get("last_cursor")?)?);
                let Json(value): Json<Value> = row.try_get("projection")?;
                Ok::<ProjectionRecord, PgStoreError>(ProjectionRecord {
                    tenant_id,
                    last_cursor,
                    projection: serde_json::from_value(value)?,
                })
            })
            .transpose()?;
        transaction.commit().await?;
        Ok(projection)
    }

    pub async fn enroll_agent(
        &self,
        tenant_id: TenantId,
        agent_id: AgentId,
        display_name: Option<String>,
    ) -> Result<AgentRecord, PgStoreError> {
        self.enroll_agent_as(tenant_id, agent_id, display_name, "client-api")
            .await
    }

    pub async fn enroll_agent_as(
        &self,
        tenant_id: TenantId,
        agent_id: AgentId,
        display_name: Option<String>,
        actor_id: &str,
    ) -> Result<AgentRecord, PgStoreError> {
        let actor_id = required_actor_id(actor_id)?;
        let display_name = normalized_display_name(display_name.as_deref());
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;

        // Agent identity is part of the tenant projection, so bound distinct
        // identities under the same tenant lock used by event appends. An
        // existing identity may still be updated when the tenant is full.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(tenant_lock_key(tenant_id))
            .execute(&mut *transaction)
            .await?;
        let cardinality = sqlx::query(AGENT_CARDINALITY_SQL)
            .bind(tenant_id.as_uuid())
            .bind(agent_id.as_str())
            .fetch_one(&mut *transaction)
            .await?;
        validate_agent_capacity(
            tenant_id,
            cardinality.try_get("agent_count")?,
            cardinality.try_get("already_enrolled")?,
        )?;

        let row = sqlx::query(
            "INSERT INTO agents (tenant_id, agent_id, display_name, enabled) \
             VALUES ($1, $2, $3, TRUE) \
             ON CONFLICT (tenant_id, agent_id) DO UPDATE \
             SET display_name = EXCLUDED.display_name, enabled = TRUE, updated_at = now() \
             RETURNING agent_id, display_name, enabled, \
                 (EXTRACT(EPOCH FROM enrolled_at) * 1000000000)::bigint \
                     AS enrolled_at_unix_nano, \
                 (EXTRACT(EPOCH FROM last_seen_at) * 1000000000)::bigint \
                     AS last_seen_at_unix_nano",
        )
        .bind(tenant_id.as_uuid())
        .bind(agent_id.as_str())
        .bind(display_name)
        .fetch_one(&mut *transaction)
        .await?;
        let agent = agent_record_from_row(&row, tenant_id)?;
        sqlx::query(
            "INSERT INTO audit_events (\
                 tenant_id, actor_type, actor_id, action, target_type, target_id, details\
             ) VALUES ($1, 'user', $2, 'agent.enroll', 'agent', $3, $4)",
        )
        .bind(tenant_id.as_uuid())
        .bind(actor_id)
        .bind(agent_id.as_str())
        .bind(Json(json!({ "display_name": display_name })))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(agent)
    }

    pub async fn set_agent_enabled(
        &self,
        tenant_id: TenantId,
        agent_id: &AgentId,
        enabled: bool,
    ) -> Result<Option<AgentRecord>, PgStoreError> {
        self.set_agent_enabled_as(tenant_id, agent_id, enabled, "client-api")
            .await
    }

    pub async fn set_agent_enabled_as(
        &self,
        tenant_id: TenantId,
        agent_id: &AgentId,
        enabled: bool,
        actor_id: &str,
    ) -> Result<Option<AgentRecord>, PgStoreError> {
        let actor_id = required_actor_id(actor_id)?;
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;
        let row = sqlx::query(
            "UPDATE agents SET enabled = $3, updated_at = now() \
             WHERE tenant_id = $1 AND agent_id = $2 \
             RETURNING agent_id, display_name, enabled, \
                 (EXTRACT(EPOCH FROM enrolled_at) * 1000000000)::bigint \
                     AS enrolled_at_unix_nano, \
                 (EXTRACT(EPOCH FROM last_seen_at) * 1000000000)::bigint \
                     AS last_seen_at_unix_nano",
        )
        .bind(tenant_id.as_uuid())
        .bind(agent_id.as_str())
        .bind(enabled)
        .fetch_optional(&mut *transaction)
        .await?;
        let agent = row
            .as_ref()
            .map(|row| agent_record_from_row(row, tenant_id))
            .transpose()?;
        if agent.is_some() {
            sqlx::query(
                "INSERT INTO audit_events (\
                     tenant_id, actor_type, actor_id, action, target_type, target_id, details\
                 ) VALUES ($1, 'user', $2, 'agent.set_enabled', 'agent', $3, $4)",
            )
            .bind(tenant_id.as_uuid())
            .bind(actor_id)
            .bind(agent_id.as_str())
            .bind(Json(json!({ "enabled": enabled })))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(agent)
    }

    /// Creates a short-lived, opaque credential that a browser may exchange
    /// exactly once when upgrading its signal stream to a WebSocket.
    ///
    /// The authenticated caller's OIDC subject is retained server-side;
    /// the bearer token itself never needs to appear in a WebSocket URL or
    /// subprotocol. The ticket expires at the earlier of `ttl` and the
    /// OIDC token's `authorized_until_unix` (`exp`) value. PostgreSQL's
    /// transaction clock rejects an authorization that is already expired.
    /// Expired and already-consumed tickets for the same tenant are
    /// opportunistically removed in the creation transaction.
    pub async fn create_stream_ticket(
        &self,
        tenant_id: TenantId,
        subject: &str,
        authorized_until_unix: u64,
        ttl: Duration,
    ) -> Result<Uuid, PgStoreError> {
        let subject = required_stream_ticket_subject(subject)?;
        let authorized_until_unix = stream_ticket_authorization_unix(authorized_until_unix)?;
        let ttl_millis = stream_ticket_ttl_millis(ttl)?;
        let ticket_id = Uuid::new_v4();
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;

        cleanup_stream_tickets_in(&mut transaction, tenant_id).await?;
        let inserted = sqlx::query(CREATE_STREAM_TICKET_SQL)
            .bind(ticket_id)
            .bind(tenant_id.as_uuid())
            .bind(subject)
            .bind(ttl_millis)
            .bind(authorized_until_unix)
            .execute(&mut *transaction)
            .await?;
        if inserted.rows_affected() == 0 {
            return Err(PgStoreError::StreamTicketAuthorizationExpired);
        }

        transaction.commit().await?;
        Ok(ticket_id)
    }

    /// Atomically consumes an unexpired ticket. Concurrent or replayed
    /// exchanges observe `None`, as do expired and cross-tenant attempts.
    pub async fn consume_stream_ticket(
        &self,
        tenant_id: TenantId,
        ticket_id: Uuid,
    ) -> Result<Option<StreamTicketGrant>, PgStoreError> {
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;
        let row = sqlx::query(CONSUME_STREAM_TICKET_SQL)
            .bind(tenant_id.as_uuid())
            .bind(ticket_id)
            .fetch_optional(&mut *transaction)
            .await?;
        let grant = row
            .map(|row| {
                Ok::<StreamTicketGrant, PgStoreError>(StreamTicketGrant {
                    subject: row.try_get("subject")?,
                    authorized_until_unix: from_pg_i64(
                        "authorized_until_unix",
                        row.try_get("authorized_until_unix")?,
                    )?,
                })
            })
            .transpose()?;
        transaction.commit().await?;
        Ok(grant)
    }

    /// Removes tickets which can no longer be exchanged for this tenant.
    pub async fn cleanup_stream_tickets(&self, tenant_id: TenantId) -> Result<u64, PgStoreError> {
        let mut transaction = self.pool.begin().await?;
        set_tenant_context(&mut transaction, tenant_id).await?;
        require_enabled_tenant(&mut transaction, tenant_id).await?;
        let removed = cleanup_stream_tickets_in(&mut transaction, tenant_id).await?;
        transaction.commit().await?;
        Ok(removed)
    }
}

impl SignalWriter for PgSignalStore {
    type Error = PgStoreError;

    fn append<'a>(
        &'a self,
        envelope: &'a SignalEnvelope,
    ) -> StoreFuture<'a, AppendOutcome, Self::Error> {
        Box::pin(async move { self.append_envelope(envelope).await })
    }
}

impl SignalReader for PgSignalStore {
    type Error = PgStoreError;

    fn load_after<'a>(
        &'a self,
        scope: SignalScope,
        after: SignalCursor,
        limit: usize,
    ) -> StoreFuture<'a, Vec<SignalRecord>, Self::Error> {
        Box::pin(async move { self.load_envelopes_after(scope, after, limit).await })
    }
}

struct CanonicalSignalPayload {
    value: Value,
    byte_len: usize,
}

struct BoundedPayloadWriter {
    bytes: Vec<u8>,
    overflowed: bool,
}

impl BoundedPayloadWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            overflowed: false,
        }
    }
}

impl Write for BoundedPayloadWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let retained_limit = MAX_STORED_SIGNAL_JSON_BYTES;
        let remaining = retained_limit.saturating_sub(self.bytes.len());
        let retained = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..retained]);
        if retained < buffer.len() {
            self.overflowed = true;
            return Err(io::Error::other("serialized signal payload limit exceeded"));
        }
        Ok(retained)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn canonical_signal_payload(payload: &Signal) -> Result<CanonicalSignalPayload, PgStoreError> {
    let encoded = serialize_signal_bounded(payload)?;

    // serde_json represents non-finite floats as null. Decode and serialize the
    // concrete Signal again so lossy conversions (and future serializer
    // asymmetry) are rejected before a database transaction begins.
    let decoded =
        serde_json::from_slice::<Signal>(&encoded).map_err(PgStoreError::InvalidSignalPayload)?;
    let round_trip = serialize_signal_bounded(&decoded)?;
    if round_trip != encoded {
        return Err(PgStoreError::SignalPayloadNotRoundTripSafe);
    }

    let value = serde_json::from_slice(&encoded).map_err(PgStoreError::InvalidSignalPayload)?;
    Ok(CanonicalSignalPayload {
        value,
        byte_len: encoded.len(),
    })
}

fn serialize_signal_bounded(payload: &Signal) -> Result<Vec<u8>, PgStoreError> {
    let mut writer = BoundedPayloadWriter::new();
    if let Err(error) = serde_json::to_writer(&mut writer, payload) {
        if writer.overflowed {
            return Err(PgStoreError::SignalPayloadTooLarge {
                actual_bytes: MAX_STORED_SIGNAL_JSON_BYTES + 1,
                max_bytes: MAX_STORED_SIGNAL_JSON_BYTES,
            });
        }
        return Err(PgStoreError::InvalidSignalPayload(error));
    }
    validate_signal_payload_size(writer.bytes.len())?;
    Ok(writer.bytes)
}

fn validate_signal_payload_size(actual_bytes: usize) -> Result<(), PgStoreError> {
    if actual_bytes > MAX_STORED_SIGNAL_JSON_BYTES {
        Err(PgStoreError::SignalPayloadTooLarge {
            actual_bytes,
            max_bytes: MAX_STORED_SIGNAL_JSON_BYTES,
        })
    } else {
        Ok(())
    }
}

async fn runtime_structure_problems(pool: &PgPool) -> Result<Vec<String>, PgStoreError> {
    let relation_names = required_relation_names();
    let relation_rows = sqlx::query(RUNTIME_RELATIONS_SQL)
        .bind(&relation_names)
        .fetch_all(pool)
        .await?;
    let relations = relation_rows
        .iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get::<String, _>("relation_name")?,
                (
                    row.try_get::<bool, _>("row_security")?,
                    row.try_get::<bool, _>("force_row_security")?,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let mut problems = Vec::new();
    for relation in REQUIRED_RELATIONS {
        if !relations.contains_key(*relation) {
            problems.push(format!("missing public.{relation} relation"));
        }
    }
    for relation in TENANT_RELATIONS {
        if let Some((row_security, force_row_security)) = relations.get(*relation)
            && (!row_security || !force_row_security)
        {
            problems.push(format!(
                "public.{relation} must enable and force row-level security"
            ));
        }
    }

    if !relations.contains_key("_sqlx_migrations") {
        return Ok(problems);
    }

    let can_read_migrations: bool = sqlx::query_scalar(MIGRATION_SELECT_PRIVILEGE_SQL)
        .fetch_one(pool)
        .await?;
    if !can_read_migrations {
        problems.push(
            "runtime role needs SELECT on public._sqlx_migrations for checksum verification"
                .to_string(),
        );
    }

    let column_rows = sqlx::query(RUNTIME_COLUMNS_SQL)
        .bind(&relation_names)
        .fetch_all(pool)
        .await?;
    let columns = column_rows
        .iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                (
                    row.try_get::<String, _>("relation_name")?,
                    row.try_get::<String, _>("column_name")?,
                ),
                (
                    row.try_get::<String, _>("data_type")?,
                    row.try_get::<bool, _>("not_null")?,
                    row.try_get::<String, _>("identity")?,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    for required in REQUIRED_COLUMNS {
        match columns.get(&(required.relation.to_string(), required.name.to_string())) {
            None => problems.push(format!(
                "missing public.{}.{} column",
                required.relation, required.name
            )),
            Some((data_type, not_null, identity))
                if data_type != required.data_type
                    || *not_null != required.not_null
                    || identity != required.identity =>
            {
                problems.push(format!(
                    "public.{}.{} has incompatible type, nullability, or identity mode",
                    required.relation, required.name
                ));
            }
            Some(_) => {}
        }
    }

    let constraint_rows = sqlx::query(RUNTIME_CONSTRAINTS_SQL)
        .bind(&relation_names)
        .fetch_all(pool)
        .await?;
    let constraints = constraint_rows
        .iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                (
                    row.try_get::<String, _>("relation_name")?,
                    row.try_get::<String, _>("constraint_name")?,
                ),
                (
                    row.try_get::<String, _>("constraint_type")?,
                    row.try_get::<bool, _>("validated")?,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    for required in REQUIRED_CONSTRAINTS {
        match constraints.get(&(required.relation.to_string(), required.name.to_string())) {
            Some((constraint_type, validated))
                if constraint_type == required.constraint_type && *validated => {}
            _ => problems.push(format!(
                "missing or incompatible {} constraint public.{}.{}",
                required.constraint_type, required.relation, required.name
            )),
        }
    }

    let index_rows = sqlx::query(RUNTIME_INDEXES_SQL)
        .bind(&relation_names)
        .fetch_all(pool)
        .await?;
    let indexes = index_rows
        .iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                (
                    row.try_get::<String, _>("relation_name")?,
                    row.try_get::<String, _>("index_name")?,
                ),
                (
                    row.try_get::<bool, _>("valid")?,
                    row.try_get::<bool, _>("ready")?,
                ),
            ))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    for (relation, index) in REQUIRED_INDEXES {
        match indexes.get(&(relation.to_string(), index.to_string())) {
            Some((valid, ready)) if *valid && *ready => {}
            _ => problems.push(format!(
                "missing, invalid, or unready index public.{index} on public.{relation}"
            )),
        }
    }

    let policy_rows = sqlx::query(RUNTIME_POLICIES_SQL)
        .bind(&relation_names)
        .fetch_all(pool)
        .await?;
    let mut policies = HashMap::new();
    for row in policy_rows {
        let relation: String = row.try_get("relation_name")?;
        let policy: String = row.try_get("policy_name")?;
        policies.insert(
            (relation, policy),
            (
                row.try_get::<bool, _>("permissive")?,
                row.try_get::<String, _>("command")?,
                row.try_get::<bool, _>("public_only")?,
                row.try_get::<Option<String>, _>("using_expression")?,
                row.try_get::<Option<String>, _>("check_expression")?,
            ),
        );
    }
    for (relation, policy_name) in policies.keys() {
        if !is_allowed_runtime_policy_name(relation, policy_name) {
            problems.push(format!(
                "public.{relation} has unexpected row-level security policy {policy_name}"
            ));
        }
    }
    for relation in TENANT_RELATIONS {
        let policy_name = expected_tenant_policy_name(relation);
        let tenant_column = if *relation == "tenants" {
            "id"
        } else {
            "tenant_id"
        };
        let expected_expression = normalize_policy_expression(&format!(
            "{tenant_column} = NULLIF(current_setting('app.tenant_id', true), '')::uuid"
        ));
        let valid = policies
            .get(&(relation.to_string(), policy_name.clone()))
            .is_some_and(
                |(permissive, command, public_only, using_expression, check_expression)| {
                    *permissive
                        && command == "*"
                        && *public_only
                        && using_expression
                            .as_deref()
                            .map(normalize_policy_expression)
                            .as_deref()
                            == Some(expected_expression.as_str())
                        && check_expression
                            .as_deref()
                            .map(normalize_policy_expression)
                            .as_deref()
                            == Some(expected_expression.as_str())
                },
            );
        if !valid {
            problems.push(format!(
                "public.{relation} is missing exact tenant isolation policy {policy_name}"
            ));
        }
    }

    Ok(problems)
}

async fn runtime_migration_problems(pool: &PgPool) -> Result<Vec<String>, PgStoreError> {
    let rows = sqlx::query(APPLIED_MIGRATIONS_SQL).fetch_all(pool).await?;
    let applied = rows
        .iter()
        .map(|row| {
            Ok::<_, sqlx::Error>((
                row.try_get::<i64, _>("version")?,
                row.try_get::<bool, _>("success")?,
                row.try_get::<Vec<u8>, _>("checksum")?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(validate_applied_migrations(&applied))
}

fn validate_applied_migrations(applied: &[(i64, bool, Vec<u8>)]) -> Vec<String> {
    let migrator = embedded_migrator();
    let expected_versions = migrator
        .migrations
        .iter()
        .map(|migration| migration.version)
        .collect::<HashSet<_>>();
    let applied_versions = applied
        .iter()
        .map(|(version, _, _)| *version)
        .collect::<HashSet<_>>();
    let mut problems = Vec::new();
    if expected_versions != applied_versions || applied.len() != migrator.migrations.len() {
        problems.push("applied migration versions do not exactly match this binary".to_string());
    }
    for migration in migrator.migrations.iter() {
        match applied
            .iter()
            .find(|(version, _, _)| *version == migration.version)
        {
            Some((_, true, checksum)) if checksum.as_slice() == migration.checksum.as_ref() => {}
            Some((_, false, _)) => problems.push(format!(
                "migration {} is recorded as unsuccessful",
                migration.version
            )),
            Some((_, true, _)) => problems.push(format!(
                "migration {} checksum does not match this binary",
                migration.version
            )),
            None => {}
        }
    }
    problems
}

fn schema_ready(problems: Vec<String>) -> Result<(), PgStoreError> {
    if problems.is_empty() {
        Ok(())
    } else {
        Err(PgStoreError::RuntimeSchemaNotReady { problems })
    }
}

fn required_relation_names() -> Vec<String> {
    REQUIRED_RELATIONS
        .iter()
        .map(|relation| (*relation).to_string())
        .collect()
}

fn normalize_policy_expression(expression: &str) -> String {
    expression
        .to_ascii_lowercase()
        .replace("::text", "")
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '(' && *character != ')')
        .collect()
}

fn expected_tenant_policy_name(relation: &str) -> String {
    format!("{relation}_tenant_isolation")
}

fn is_allowed_runtime_policy_name(relation: &str, policy_name: &str) -> bool {
    !TENANT_RELATIONS.contains(&relation)
        || policy_name == expected_tenant_policy_name(relation).as_str()
}

fn embedded_migrator() -> Migrator {
    let mut migrator =
        Migrator::with_migrations(EMBEDDED_MIGRATIONS.migrations.iter().cloned().collect());
    migrator.dangerous_set_table_name("public._sqlx_migrations");
    migrator
}

async fn set_tenant_context(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
) -> Result<(), PgStoreError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn require_enabled_tenant(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
) -> Result<(), PgStoreError> {
    let enabled = sqlx::query(REQUIRE_ENABLED_TENANT_SQL)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await?
        .map(|row| row.get::<bool, _>("enabled"));
    match enabled {
        Some(true) => Ok(()),
        Some(false) => Err(PgStoreError::TenantDisabled { tenant_id }),
        None => Err(PgStoreError::Database(sqlx::Error::RowNotFound)),
    }
}

async fn cleanup_stream_tickets_in(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
) -> Result<u64, PgStoreError> {
    let result = sqlx::query(
        "DELETE FROM stream_tickets WHERE tenant_id = $1 \
         AND (consumed_at IS NOT NULL OR expires_at <= statement_timestamp())",
    )
    .bind(tenant_id.as_uuid())
    .execute(&mut **transaction)
    .await?;
    Ok(result.rows_affected())
}

fn tenant_scope(scope: SignalScope) -> Result<TenantId, PgStoreError> {
    match scope {
        SignalScope::Tenant { tenant_id } => Ok(tenant_id),
        SignalScope::Solo => Err(PgStoreError::SoloScope),
    }
}

fn signal_kind_name(envelope: &SignalEnvelope) -> &'static str {
    match envelope.kind() {
        skid_monitor_core::SignalKind::Metrics => "metrics",
        skid_monitor_core::SignalKind::Traces => "traces",
        skid_monitor_core::SignalKind::Logs => "logs",
    }
}

fn bounded_load_limit(limit: usize) -> i64 {
    limit.min(MAX_LOAD_LIMIT) as i64
}

fn bounded_replay_json_bytes(max_json_bytes: usize) -> i64 {
    max_json_bytes.min(MAX_REPLAY_JSON_BYTES) as i64
}

fn validate_runtime_role(capabilities: RuntimeRoleCapabilities) -> Result<(), PgStoreError> {
    let mut reasons = Vec::new();
    for (unsafe_capability, reason) in [
        (capabilities.session_user_differs, "session_user differs"),
        (capabilities.superuser, "SUPERUSER"),
        (capabilities.bypasses_rls, "BYPASSRLS"),
        (capabilities.creates_database, "CREATEDB"),
        (capabilities.creates_role, "CREATEROLE"),
        (capabilities.replication, "REPLICATION"),
        (
            capabilities.can_create_public_schema,
            "CREATE on public schema",
        ),
        (
            capabilities.member_of_relation_owner,
            "application relation owner membership",
        ),
        (
            capabilities.member_of_privileged_role,
            "privileged role membership",
        ),
    ] {
        if unsafe_capability {
            reasons.push(reason);
        }
    }
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(PgStoreError::UnsafeRuntimeRole {
            role_name: capabilities.role_name,
            reasons,
        })
    }
}

fn validate_agent_capacity(
    tenant_id: TenantId,
    agent_count: i64,
    already_enrolled: bool,
) -> Result<(), PgStoreError> {
    if !already_enrolled && agent_count >= MAX_AGENTS_PER_TENANT as i64 {
        Err(PgStoreError::AgentLimitReached {
            tenant_id,
            max_agents: MAX_AGENTS_PER_TENANT,
        })
    } else {
        Ok(())
    }
}

fn normalized_display_name(display_name: Option<&str>) -> Option<&str> {
    display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn required_actor_id(actor_id: &str) -> Result<&str, PgStoreError> {
    let actor_id = actor_id.trim();
    if actor_id.is_empty() {
        Err(PgStoreError::InvalidActorId)
    } else {
        Ok(actor_id)
    }
}

fn required_stream_ticket_subject(subject: &str) -> Result<&str, PgStoreError> {
    let subject = subject.trim();
    if subject.is_empty() || subject.len() > MAX_STREAM_TICKET_SUBJECT_BYTES {
        Err(PgStoreError::InvalidStreamTicketSubject)
    } else {
        Ok(subject)
    }
}

fn stream_ticket_ttl_millis(ttl: Duration) -> Result<i64, PgStoreError> {
    if ttl.is_zero() || ttl > MAX_STREAM_TICKET_TTL {
        return Err(PgStoreError::InvalidStreamTicketTtl);
    }
    let millis = ttl.as_millis();
    if millis == 0 {
        return Err(PgStoreError::InvalidStreamTicketTtl);
    }
    i64::try_from(millis).map_err(|_| PgStoreError::InvalidStreamTicketTtl)
}

fn stream_ticket_authorization_unix(authorized_until_unix: u64) -> Result<i64, PgStoreError> {
    if authorized_until_unix > MAX_STREAM_TICKET_AUTHORIZATION_UNIX {
        return Err(PgStoreError::InvalidStreamTicketAuthorizationExpiry);
    }
    i64::try_from(authorized_until_unix)
        .map_err(|_| PgStoreError::InvalidStreamTicketAuthorizationExpiry)
}

fn tenant_lock_key(tenant_id: TenantId) -> i64 {
    let bytes = tenant_id.as_uuid().into_bytes();
    let high = u64::from_be_bytes(bytes[..8].try_into().expect("UUID high half"));
    let low = u64::from_be_bytes(bytes[8..].try_into().expect("UUID low half"));
    (high ^ low) as i64
}

fn to_pg_i64(field: &'static str, value: u64) -> Result<i64, PgStoreError> {
    i64::try_from(value).map_err(|_| PgStoreError::IntegerOutOfRange { field })
}

fn from_pg_i64(field: &'static str, value: i64) -> Result<u64, PgStoreError> {
    u64::try_from(value).map_err(|_| PgStoreError::IntegerOutOfRange { field })
}

fn from_pg_u64_text(field: &'static str, value: String) -> Result<u64, PgStoreError> {
    value.parse().map_err(|_| PgStoreError::CorruptRow(field))
}

fn cursor_from_row(row: &PgRow) -> Result<SignalCursor, PgStoreError> {
    Ok(SignalCursor(from_pg_i64("cursor", row.try_get("cursor")?)?))
}

fn signal_record_from_row(row: &PgRow, tenant_id: TenantId) -> Result<SignalRecord, PgStoreError> {
    let event_id: Uuid = row.try_get("event_id")?;
    let agent_id: String = row.try_get("agent_id")?;
    let Json(payload): Json<Value> = row.try_get("payload")?;
    Ok(SignalRecord {
        cursor: cursor_from_row(row)?,
        envelope: SignalEnvelope {
            event_id: event_id
                .to_string()
                .parse::<EventId>()
                .map_err(|_| PgStoreError::CorruptRow("event_id"))?,
            scope: SignalScope::tenant(tenant_id),
            agent_id: AgentId::new(agent_id)
                .map_err(|error| PgStoreError::InvalidAgentId(error.to_string()))?,
            sequence: from_pg_u64_text("sequence", row.try_get("sequence")?)?,
            received_at_unix_nano: from_pg_u64_text(
                "received_at_unix_nano",
                row.try_get("received_at_unix_nano")?,
            )?,
            payload: serde_json::from_value(payload)?,
        },
    })
}

fn agent_record_from_row(row: &PgRow, tenant_id: TenantId) -> Result<AgentRecord, PgStoreError> {
    let agent_id: String = row.try_get("agent_id")?;
    let enrolled_at: i64 = row.try_get("enrolled_at_unix_nano")?;
    let last_seen_at: Option<i64> = row.try_get("last_seen_at_unix_nano")?;
    Ok(AgentRecord {
        tenant_id,
        agent_id: AgentId::new(agent_id)
            .map_err(|error| PgStoreError::InvalidAgentId(error.to_string()))?,
        display_name: row.try_get("display_name")?,
        enabled: row.try_get("enabled")?,
        enrolled_at_unix_nano: from_pg_i64("enrolled_at_unix_nano", enrolled_at)?,
        last_seen_at_unix_nano: last_seen_at
            .map(|value| from_pg_i64("last_seen_at_unix_nano", value))
            .transpose()?,
    })
}

fn tenant_record_from_row(row: &PgRow, tenant_id: TenantId) -> Result<TenantRecord, PgStoreError> {
    let created_at: i64 = row.try_get("created_at_unix_nano")?;
    Ok(TenantRecord {
        tenant_id,
        slug: row.try_get("slug")?,
        display_name: row.try_get("display_name")?,
        enabled: row.try_get("enabled")?,
        created_at_unix_nano: from_pg_i64("created_at_unix_nano", created_at)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};

    fn safe_runtime_role(role_name: &str) -> RuntimeRoleCapabilities {
        RuntimeRoleCapabilities {
            role_name: role_name.to_string(),
            session_user_differs: false,
            superuser: false,
            bypasses_rls: false,
            creates_database: false,
            creates_role: false,
            replication: false,
            can_create_public_schema: false,
            member_of_relation_owner: false,
            member_of_privileged_role: false,
        }
    }

    #[test]
    fn pool_options_reject_unbounded_or_inverted_limits() {
        let options = PgStoreOptions {
            max_connections: 0,
            ..PgStoreOptions::default()
        };
        assert!(matches!(
            options.validate(),
            Err(PgStoreError::InvalidPoolOptions(_))
        ));

        let defaults = PgStoreOptions::default();
        let options = PgStoreOptions {
            min_connections: defaults.max_connections + 1,
            ..defaults
        };
        assert!(matches!(
            options.validate(),
            Err(PgStoreError::InvalidPoolOptions(_))
        ));
    }

    #[test]
    fn load_limit_is_bounded_and_zero_is_preserved() {
        assert_eq!(bounded_load_limit(0), 0);
        assert_eq!(bounded_load_limit(7), 7);
        assert_eq!(bounded_load_limit(usize::MAX), MAX_LOAD_LIMIT as i64);
    }

    #[test]
    fn replay_json_byte_limit_is_bounded_and_zero_allows_the_first_row_only() {
        assert_eq!(bounded_replay_json_bytes(0), 0);
        assert_eq!(bounded_replay_json_bytes(7), 7);
        assert_eq!(
            bounded_replay_json_bytes(usize::MAX),
            MAX_REPLAY_JSON_BYTES as i64
        );
    }

    #[test]
    fn stored_signal_payload_size_is_hard_bounded() {
        assert!(validate_signal_payload_size(MAX_STORED_SIGNAL_JSON_BYTES).is_ok());
        assert!(matches!(
            validate_signal_payload_size(MAX_STORED_SIGNAL_JSON_BYTES + 1),
            Err(PgStoreError::SignalPayloadTooLarge {
                actual_bytes,
                max_bytes: MAX_STORED_SIGNAL_JSON_BYTES,
            }) if actual_bytes == MAX_STORED_SIGNAL_JSON_BYTES + 1
        ));
    }

    #[test]
    fn payload_writer_never_buffers_beyond_the_hard_limit() {
        let mut writer = BoundedPayloadWriter::new();
        let chunk = [b'x'; 4096];
        while writer.write_all(&chunk).is_ok() {}
        assert!(writer.overflowed);
        assert_eq!(writer.bytes.len(), MAX_STORED_SIGNAL_JSON_BYTES);
    }

    #[test]
    fn canonical_payload_round_trip_rejects_non_finite_metric_values() {
        let invalid = metric_signal(f64::NAN);
        assert!(matches!(
            canonical_signal_payload(&invalid),
            Err(PgStoreError::SignalPayloadNotRoundTripSafe)
        ));

        let valid = metric_signal(12.5);
        let canonical = canonical_signal_payload(&valid).expect("finite metric is JSON-safe");
        assert_eq!(
            canonical.byte_len,
            serde_json::to_vec(&canonical.value).unwrap().len()
        );
        assert!(serde_json::from_value::<Signal>(canonical.value).is_ok());
    }

    #[test]
    fn replay_query_sizes_row_limited_metadata_before_loading_payloads() {
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("candidate_metadata AS MATERIALIZED"));
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("SELECT cursor, payload_bytes"));
        assert!(!LOAD_ENVELOPES_BOUNDED_SQL.contains("octet_length(payload::text)"));
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("LIMIT $3"));
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("sum(payload_bytes) OVER"));
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("ordinal = 1"));
        assert!(LOAD_ENVELOPES_BOUNDED_SQL.contains("JOIN cutoff"));

        let cutoff = LOAD_ENVELOPES_BOUNDED_SQL.find("cutoff AS").unwrap();
        let full_payload = LOAD_ENVELOPES_BOUNDED_SQL.find("events.payload").unwrap();
        assert!(full_payload > cutoff);
    }

    #[test]
    fn tenant_enabled_check_holds_a_lock_for_the_operation() {
        assert!(REQUIRE_ENABLED_TENANT_SQL.contains("SELECT enabled FROM tenants"));
        assert!(REQUIRE_ENABLED_TENANT_SQL.ends_with("FOR SHARE"));
    }

    #[test]
    fn runtime_role_rejects_every_rls_bypass_path() {
        assert!(validate_runtime_role(safe_runtime_role("runtime")).is_ok());

        let rejected = |capabilities, expected_reason| {
            let Err(PgStoreError::UnsafeRuntimeRole { reasons, .. }) =
                validate_runtime_role(capabilities)
            else {
                panic!("unsafe runtime role was accepted");
            };
            assert!(reasons.contains(&expected_reason), "{reasons:?}");
        };

        let mut capabilities = safe_runtime_role("set-role-session");
        capabilities.session_user_differs = true;
        rejected(capabilities, "session_user differs");

        let mut capabilities = safe_runtime_role("postgres");
        capabilities.superuser = true;
        rejected(capabilities, "SUPERUSER");

        let mut capabilities = safe_runtime_role("bypass");
        capabilities.bypasses_rls = true;
        rejected(capabilities, "BYPASSRLS");

        let mut capabilities = safe_runtime_role("createdb");
        capabilities.creates_database = true;
        rejected(capabilities, "CREATEDB");

        let mut capabilities = safe_runtime_role("createrole");
        capabilities.creates_role = true;
        rejected(capabilities, "CREATEROLE");

        let mut capabilities = safe_runtime_role("replication");
        capabilities.replication = true;
        rejected(capabilities, "REPLICATION");

        let mut capabilities = safe_runtime_role("schema-ddl");
        capabilities.can_create_public_schema = true;
        rejected(capabilities, "CREATE on public schema");

        let mut capabilities = safe_runtime_role("owner-member");
        capabilities.member_of_relation_owner = true;
        rejected(capabilities, "application relation owner membership");

        let mut capabilities = safe_runtime_role("privileged-member");
        capabilities.member_of_privileged_role = true;
        rejected(capabilities, "privileged role membership");

        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("pg_catalog.pg_roles"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("session_user <> current_user"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("rolsuper"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("rolbypassrls"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("rolcreatedb"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("rolcreaterole"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("rolreplication"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("has_schema_privilege"));
        assert!(VERIFY_RUNTIME_ROLE_SQL.contains("pg_has_role"));
    }

    #[test]
    fn runtime_schema_check_is_public_schema_and_catalog_exact() {
        assert!(SET_SAFE_SEARCH_PATH_SQL.contains("'pg_catalog, public'"));
        assert!(SET_SAFE_SEARCH_PATH_SQL.contains("pg_catalog.set_config"));
        for query in [
            RUNTIME_RELATIONS_SQL,
            RUNTIME_COLUMNS_SQL,
            RUNTIME_CONSTRAINTS_SQL,
            RUNTIME_INDEXES_SQL,
            RUNTIME_POLICIES_SQL,
        ] {
            assert!(query.contains("pg_catalog"));
            assert!(query.contains("namespaces.nspname = 'public'"));
        }
        assert!(APPLIED_MIGRATIONS_SQL.contains("public._sqlx_migrations"));
        assert!(MIGRATION_SELECT_PRIVILEGE_SQL.contains("public._sqlx_migrations"));
        assert!(RUNTIME_CONSTRAINTS_SQL.contains("constraints.convalidated"));
        assert!(REQUIRED_RELATIONS.contains(&"_sqlx_migrations"));
        assert_eq!(TENANT_RELATIONS.len(), 6);
        assert!(REQUIRED_COLUMNS.iter().any(|column| {
            column.relation == "signal_events"
                && column.name == "payload_bytes"
                && column.data_type == "bigint"
                && column.not_null
        }));
        assert!(REQUIRED_CONSTRAINTS.iter().any(|constraint| {
            constraint.relation == "signal_events"
                && constraint.name == "signal_events_payload_bytes_bounded"
                && constraint.constraint_type == "c"
        }));
        assert!(REQUIRED_INDEXES.contains(&("signal_events", "signal_events_tenant_cursor_idx")));
    }

    #[test]
    fn exact_migration_versions_and_checksums_are_required() {
        let migrator = embedded_migrator();
        let expected = migrator.migrations.first().unwrap();
        let applied = vec![(expected.version, true, expected.checksum.to_vec())];
        assert!(validate_applied_migrations(&applied).is_empty());

        let mut wrong_checksum = applied.clone();
        wrong_checksum[0].2[0] ^= 0xff;
        assert!(
            validate_applied_migrations(&wrong_checksum)
                .iter()
                .any(|problem| problem.contains("checksum"))
        );

        let mut unexpected = applied;
        unexpected.push((99, true, vec![0; 48]));
        assert!(
            validate_applied_migrations(&unexpected)
                .iter()
                .any(|problem| problem.contains("versions"))
        );
    }

    #[test]
    fn embedded_migration_inventory_is_directory_driven_and_ordered() {
        let migrator = embedded_migrator();
        let versions = migrator
            .migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>();

        assert!(!versions.is_empty());
        assert_eq!(versions[0], 1);
        assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(migrator.table_name.as_ref(), "public._sqlx_migrations");
    }

    #[test]
    fn tenant_policy_expression_must_be_exact_after_postgres_normalization() {
        let expected = normalize_policy_expression(
            "tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid",
        );
        let postgres = normalize_policy_expression(
            "(tenant_id = (NULLIF(current_setting('app.tenant_id'::text, true), ''::text))::uuid)",
        );
        let permissive = normalize_policy_expression(
            "tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid OR true",
        );
        assert_eq!(postgres, expected);
        assert_ne!(permissive, expected);
    }

    #[test]
    fn tenant_policy_set_rejects_additional_policy_names() {
        for relation in TENANT_RELATIONS {
            assert!(is_allowed_runtime_policy_name(
                relation,
                &expected_tenant_policy_name(relation)
            ));
            assert!(!is_allowed_runtime_policy_name(
                relation,
                &format!("{relation}_allow_all")
            ));
        }
        assert!(is_allowed_runtime_policy_name(
            "_sqlx_migrations",
            "operational_policy"
        ));
    }

    #[test]
    fn runtime_schema_problems_fail_closed() {
        assert!(schema_ready(Vec::new()).is_ok());
        assert!(matches!(
            schema_ready(vec!["missing policy".to_string()]),
            Err(PgStoreError::RuntimeSchemaNotReady { problems })
                if problems == ["missing policy"]
        ));
    }

    #[test]
    fn display_names_are_trimmed_and_empty_values_are_removed() {
        assert_eq!(normalized_display_name(Some(" agent ")), Some("agent"));
        assert_eq!(normalized_display_name(Some("  ")), None);
        assert_eq!(normalized_display_name(None), None);
    }

    #[test]
    fn postgres_bigint_mapping_is_checked() {
        assert_eq!(to_pg_i64("cursor", i64::MAX as u64).unwrap(), i64::MAX);
        assert!(to_pg_i64("cursor", i64::MAX as u64 + 1).is_err());
        assert!(from_pg_i64("cursor", -1).is_err());
    }

    #[test]
    fn postgres_numeric_mapping_accepts_the_full_u64_domain() {
        assert_eq!(
            from_pg_u64_text("sequence", u64::MAX.to_string()).unwrap(),
            u64::MAX
        );
        assert!(from_pg_u64_text("sequence", "-1".to_string()).is_err());
    }

    #[test]
    fn tenant_lock_key_is_stable_and_uses_the_whole_uuid() {
        let first =
            TenantId::from_uuid(Uuid::parse_str("00000000-0000-0001-0000-000000000002").unwrap());
        let second =
            TenantId::from_uuid(Uuid::parse_str("00000000-0000-0001-0000-000000000003").unwrap());
        assert_eq!(tenant_lock_key(first), tenant_lock_key(first));
        assert_ne!(tenant_lock_key(first), tenant_lock_key(second));
    }

    #[test]
    fn agent_capacity_allows_updates_but_bounds_new_identities() {
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());
        assert!(
            validate_agent_capacity(tenant_id, MAX_AGENTS_PER_TENANT as i64 - 1, false).is_ok()
        );
        assert!(validate_agent_capacity(tenant_id, MAX_AGENTS_PER_TENANT as i64, true).is_ok());
        assert!(matches!(
            validate_agent_capacity(tenant_id, MAX_AGENTS_PER_TENANT as i64, false),
            Err(PgStoreError::AgentLimitReached {
                max_agents: MAX_AGENTS_PER_TENANT,
                ..
            })
        ));
        assert!(AGENT_CARDINALITY_SQL.contains("FILTER (WHERE agent_id = $2)"));
    }

    #[test]
    fn audit_actor_id_must_not_be_empty() {
        assert_eq!(required_actor_id(" subject ").unwrap(), "subject");
        assert!(matches!(
            required_actor_id("  "),
            Err(PgStoreError::InvalidActorId)
        ));
    }

    #[test]
    fn stream_ticket_subject_is_trimmed_and_bounded() {
        assert_eq!(
            required_stream_ticket_subject(" oidc-subject ").unwrap(),
            "oidc-subject"
        );
        assert!(matches!(
            required_stream_ticket_subject("  "),
            Err(PgStoreError::InvalidStreamTicketSubject)
        ));
        let oversized = "s".repeat(MAX_STREAM_TICKET_SUBJECT_BYTES + 1);
        assert!(matches!(
            required_stream_ticket_subject(&oversized),
            Err(PgStoreError::InvalidStreamTicketSubject)
        ));
    }

    #[test]
    fn stream_ticket_ttl_is_positive_precise_and_bounded() {
        assert!(stream_ticket_ttl_millis(Duration::ZERO).is_err());
        assert!(stream_ticket_ttl_millis(Duration::from_nanos(1)).is_err());
        assert_eq!(
            stream_ticket_ttl_millis(Duration::from_millis(1)).unwrap(),
            1
        );
        assert_eq!(
            stream_ticket_ttl_millis(MAX_STREAM_TICKET_TTL).unwrap(),
            300_000
        );
        assert!(
            stream_ticket_ttl_millis(MAX_STREAM_TICKET_TTL + Duration::from_millis(1)).is_err()
        );
    }

    #[test]
    fn stream_ticket_consume_statement_is_one_time_and_expiry_guarded() {
        assert!(CONSUME_STREAM_TICKET_SQL.contains("consumed_at IS NULL"));
        assert!(CONSUME_STREAM_TICKET_SQL.contains("expires_at > statement_timestamp()"));
        assert!(CONSUME_STREAM_TICKET_SQL.starts_with("UPDATE stream_tickets"));
    }

    #[test]
    fn stream_ticket_is_capped_by_authorization_expiry() {
        assert!(CREATE_STREAM_TICKET_SQL.contains("LEAST("));
        assert!(
            CREATE_STREAM_TICKET_SQL
                .contains("authorization.authorized_until > statement_timestamp()")
        );
        assert!(CONSUME_STREAM_TICKET_SQL.contains("authorized_until_unix"));
    }

    #[test]
    fn stream_ticket_authorization_expiry_is_portably_representable() {
        assert_eq!(
            stream_ticket_authorization_unix(MAX_STREAM_TICKET_AUTHORIZATION_UNIX).unwrap(),
            MAX_STREAM_TICKET_AUTHORIZATION_UNIX as i64
        );
        assert!(matches!(
            stream_ticket_authorization_unix(MAX_STREAM_TICKET_AUTHORIZATION_UNIX + 1),
            Err(PgStoreError::InvalidStreamTicketAuthorizationExpiry)
        ));
    }

    fn metric_signal(value: f64) -> Signal {
        Signal::Metrics(export_metrics(
            vec![Metric {
                name: "system.cpu.usage".to_string(),
                value,
                source: Source::System,
                unit: Some("%".to_string()),
                kind: MetricKind::Gauge,
                attributes: Vec::new(),
            }],
            "store-test",
            "store-test",
        ))
    }
}
