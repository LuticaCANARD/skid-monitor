use skid_monitor_core::{AgentId, SignalCursor, SignalEnvelope, SignalScope, TenantId};
use skid_monitor_server::store::{PgSignalStore, PgStoreError, PgStoreOptions};
use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};
use skid_protocol::protocol::Signal;
use sqlx::{AssertSqlSafe, PgPool, Row};
use std::error::Error;
use std::io;
use uuid::Uuid;

const TEST_DATABASE_URL_ENV: &str = "SKID_MONITOR_TEST_DATABASE_URL";

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires an explicitly provisioned PostgreSQL database"]
async fn postgres_store_is_idempotent_projected_and_tenant_isolated() {
    let Ok(database_url) = std::env::var(TEST_DATABASE_URL_ENV) else {
        eprintln!(
            "skipping PostgreSQL integration test: {TEST_DATABASE_URL_ENV} is not configured"
        );
        return;
    };

    let store = PgSignalStore::connect(&database_url, PgStoreOptions::default())
        .await
        .expect("connect to PostgreSQL integration-test database");
    store
        .migrate()
        .await
        .expect("migrate PostgreSQL integration-test database");
    store
        .verify_runtime_schema()
        .await
        .expect("verify exact PostgreSQL integration-test schema");

    let tenant_a = TenantId::from_uuid(Uuid::new_v4());
    let tenant_b = TenantId::from_uuid(Uuid::new_v4());
    let scenario = run_scenario(&store, tenant_a, tenant_b).await;
    let cleanup = cleanup_tenants(store.pool(), &[tenant_a, tenant_b]).await;

    match (scenario, cleanup) {
        (Ok(()), Ok(())) => {}
        (Err(scenario), Ok(())) => panic!("PostgreSQL store scenario failed: {scenario}"),
        (Ok(()), Err(cleanup)) => panic!("PostgreSQL store cleanup failed: {cleanup}"),
        (Err(scenario), Err(cleanup)) => {
            panic!("PostgreSQL store scenario failed: {scenario}; cleanup also failed: {cleanup}")
        }
    }
}

async fn run_scenario(store: &PgSignalStore, tenant_a: TenantId, tenant_b: TenantId) -> TestResult {
    let suffix_a = tenant_a.as_uuid().simple().to_string();
    let suffix_b = tenant_b.as_uuid().simple().to_string();
    store
        .upsert_tenant(
            tenant_a,
            format!("integration-a-{suffix_a}"),
            "Integration tenant A".to_string(),
        )
        .await?;
    store
        .upsert_tenant(
            tenant_b,
            format!("integration-b-{suffix_b}"),
            "Integration tenant B".to_string(),
        )
        .await?;

    let agent_id = AgentId::new(format!("integration-agent-{suffix_a}"))?;
    store
        .enroll_agent(
            tenant_a,
            agent_id.clone(),
            Some("PostgreSQL integration agent".to_string()),
        )
        .await?;

    let envelope = metric_envelope(tenant_a, agent_id.clone(), 17, 12.5);
    let inserted = store.append_envelope(&envelope).await?;
    require(inserted.inserted, "the first append must insert an event")?;

    let duplicate = store.append_envelope(&envelope).await?;
    require(
        !duplicate.inserted,
        "the same sequence and payload must be idempotent",
    )?;
    require(
        duplicate.cursor == inserted.cursor,
        "an idempotent append must return the original cursor",
    )?;

    let conflict = metric_envelope(tenant_a, agent_id.clone(), 17, 99.0);
    match store.append_envelope(&conflict).await {
        Err(PgStoreError::SequenceConflict {
            agent_id: conflicting_agent,
            sequence: 17,
        }) if conflicting_agent == agent_id => {}
        Err(error) => {
            return Err(test_error(format!(
                "different payload at the same sequence returned the wrong error: {error}"
            )));
        }
        Ok(outcome) => {
            return Err(test_error(format!(
                "different payload at the same sequence unexpectedly appended at cursor {}",
                outcome.cursor.0
            )));
        }
    }

    let non_finite = metric_envelope(tenant_a, agent_id.clone(), 18, f64::NAN);
    match store.append_envelope(&non_finite).await {
        Err(
            PgStoreError::InvalidSignalPayload(_) | PgStoreError::SignalPayloadNotRoundTripSafe,
        ) => {}
        Err(error) => {
            return Err(test_error(format!(
                "non-finite metric returned the wrong error: {error}"
            )));
        }
        Ok(outcome) => {
            return Err(test_error(format!(
                "non-finite metric unexpectedly appended at cursor {}",
                outcome.cursor.0
            )));
        }
    }

    let replay = store
        .load_envelopes_after(SignalScope::tenant(tenant_a), SignalCursor(0), 10)
        .await?;
    require(replay.len() == 1, "tenant A must replay exactly one event")?;
    require(
        replay[0].cursor == inserted.cursor,
        "replay must retain the committed cursor",
    )?;
    require(
        serde_json::to_value(&replay[0].envelope)? == serde_json::to_value(&envelope)?,
        "replay must retain the canonical SignalEnvelope",
    )?;

    let projection = store
        .load_projection(tenant_a)
        .await?
        .ok_or_else(|| test_error("tenant A projection was not created"))?;
    require(
        projection.last_cursor == inserted.cursor,
        "projection cursor must match the inserted signal",
    )?;
    require(
        projection.projection.counters.metric_batches == 1
            && projection.projection.counters.metric_points == 1,
        "an idempotent replay must not increment tenant projection counters",
    )?;
    let agent_projection = projection
        .projection
        .agents
        .get(&agent_id)
        .ok_or_else(|| test_error("projection is missing the enrolled agent"))?;
    require(
        agent_projection.last_sequence == envelope.sequence
            && agent_projection.counters.metric_batches == 1
            && agent_projection.counters.metric_points == 1,
        "agent projection must reflect the canonical metrics envelope exactly once",
    )?;

    let tenant_b_replay = store
        .load_envelopes_after(SignalScope::tenant(tenant_b), SignalCursor(0), 10)
        .await?;
    require(
        tenant_b_replay.is_empty(),
        "tenant B must not replay tenant A events",
    )?;

    let directly_visible = direct_signal_count(store.pool(), tenant_b, tenant_a).await?;
    require(
        directly_visible == 0,
        "PostgreSQL RLS exposed tenant A signal_events under tenant B context",
    )?;

    Ok(())
}

fn metric_envelope(
    tenant_id: TenantId,
    agent_id: AgentId,
    sequence: u64,
    value: f64,
) -> SignalEnvelope {
    let payload = Signal::Metrics(export_metrics(
        vec![Metric {
            name: "system.cpu.usage".to_string(),
            value,
            source: Source::System,
            unit: Some("%".to_string()),
            kind: MetricKind::Gauge,
            attributes: vec![("test.case".to_string(), "postgres-store".to_string())],
        }],
        "integration-agent",
        "postgres-store-test",
    ));
    SignalEnvelope::new(
        SignalScope::tenant(tenant_id),
        agent_id,
        sequence,
        1_725_000_000_000_000_000,
        payload,
    )
}

async fn direct_signal_count(
    pool: &PgPool,
    viewing_tenant: TenantId,
    target_tenant: TenantId,
) -> TestResult<i64> {
    let role =
        sqlx::query("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(pool)
            .await?;
    let is_superuser: bool = role.try_get("rolsuper")?;
    let bypasses_rls: bool = role.try_get("rolbypassrls")?;

    if !bypasses_rls {
        return direct_signal_count_as_current_role(pool, viewing_tenant, target_tenant).await;
    }
    if !is_superuser {
        return Err(test_error(
            "the integration-test database role has BYPASSRLS; use a non-bypass role so RLS can be verified",
        ));
    }

    // Docker-based PostgreSQL tests commonly connect as a superuser. Superusers
    // always bypass RLS, so exercise the same query through a short-lived,
    // non-login role instead of producing a false positive.
    let restricted_role = format!("skid_rls_test_{}", Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!(
        "CREATE ROLE {restricted_role} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS"
    )))
    .execute(pool)
    .await?;

    let scenario = async {
        sqlx::query(AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA public TO {restricted_role}"
        )))
        .execute(pool)
        .await?;
        sqlx::query(AssertSqlSafe(format!(
            "GRANT SELECT ON TABLE signal_events TO {restricted_role}"
        )))
        .execute(pool)
        .await?;

        let mut transaction = pool.begin().await?;
        sqlx::query(AssertSqlSafe(format!("SET LOCAL ROLE {restricted_role}")))
            .execute(&mut *transaction)
            .await?;
        set_tenant_context(&mut transaction, viewing_tenant).await?;
        let count =
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM signal_events WHERE tenant_id = $1")
                .bind(target_tenant.as_uuid())
                .fetch_one(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok::<i64, Box<dyn Error + Send + Sync>>(count)
    }
    .await;

    let role_cleanup = cleanup_restricted_role(pool, &restricted_role).await;
    match (scenario, role_cleanup) {
        (Ok(count), Ok(())) => Ok(count),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(test_error(format!(
            "RLS query failed: {error}; temporary role cleanup also failed: {cleanup}"
        ))),
    }
}

async fn direct_signal_count_as_current_role(
    pool: &PgPool,
    viewing_tenant: TenantId,
    target_tenant: TenantId,
) -> TestResult<i64> {
    let mut transaction = pool.begin().await?;
    set_tenant_context(&mut transaction, viewing_tenant).await?;
    let count =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM signal_events WHERE tenant_id = $1")
            .bind(target_tenant.as_uuid())
            .fetch_one(&mut *transaction)
            .await?;
    transaction.commit().await?;
    Ok(count)
}

async fn cleanup_restricted_role(pool: &PgPool, role: &str) -> TestResult {
    let revoke_table = sqlx::query(AssertSqlSafe(format!(
        "REVOKE ALL PRIVILEGES ON TABLE signal_events FROM {role}"
    )))
    .execute(pool)
    .await;
    let revoke_schema = sqlx::query(AssertSqlSafe(format!(
        "REVOKE ALL PRIVILEGES ON SCHEMA public FROM {role}"
    )))
    .execute(pool)
    .await;
    let drop_role = sqlx::query(AssertSqlSafe(format!("DROP ROLE {role}")))
        .execute(pool)
        .await;

    revoke_table?;
    revoke_schema?;
    drop_role?;
    Ok(())
}

async fn cleanup_tenants(pool: &PgPool, tenants: &[TenantId]) -> TestResult {
    let mut first_error: Option<Box<dyn Error + Send + Sync>> = None;
    for tenant_id in tenants {
        let cleanup = async {
            let mut transaction = pool.begin().await?;
            set_tenant_context(&mut transaction, *tenant_id).await?;
            sqlx::query("DELETE FROM tenants WHERE id = $1")
                .bind(tenant_id.as_uuid())
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            Ok::<(), Box<dyn Error + Send + Sync>>(())
        }
        .await;
        if let Err(error) = cleanup
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }

    first_error.map_or(Ok(()), Err)
}

async fn set_tenant_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: TenantId,
) -> TestResult {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn require(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(test_error(message))
    }
}

fn test_error(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(io::Error::other(message.into()))
}
