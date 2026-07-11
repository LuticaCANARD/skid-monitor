# Cloud OTLP export

`skid-monitor-agent` can send metrics, traces, and logs to the cloud ingress
with an OAuth 2.0 client-credentials token issued by the configured OIDC/OAuth
provider. The existing
unauthenticated OTLP exporter remains available for trusted local deployments.

Use [`examples/agent-cloud-config.json`](examples/agent-cloud-config.json) as a
starting point, then start the agent with the configuration path and the
provider client secret in separate environment variables:

```sh
export SKID_MONITOR_AGENT_CONFIG=/etc/skid-monitor/agent-cloud-config.json
export SKID_MONITOR_OIDC_CLIENT_SECRET='read-from-your-secret-store'
cargo run -p skid-monitor-agent
```

The JSON configuration contains only the name of the secret environment
variable. A `client_secret` JSON field is rejected. The token URL and the
authenticated OTLP endpoint must both use HTTPS; redirects from the token
endpoint are not followed. Public WebPKI roots are used to validate the OTLP
server certificate. Token responses are capped at 1 MiB even when the body uses
chunked transfer encoding, and `expires_in` must be between one second and 24
hours.

For Keycloak, create a confidential client with service accounts enabled. Its
client ID becomes the cloud agent identity. Configure Keycloak token mappers so
the access token has the ingress audience, a UUID `tenant_id` claim, and the
ingress role expected by the server (normally `telemetry-ingest`) under that
audience's client roles. Audience assignment is a Keycloak client-scope or
audience-mapper concern, not a value trusted from the agent JSON file.
For another provider, create its client-credentials application/service
principal and configure the generic claim pointers described in
[`../docs/oidc-account-providers.md`](../docs/oidc-account-providers.md).

The exporter obtains one token for all three signal kinds, caches it, and
refreshes it before `expires_in`. Every authenticated exporter must also set a
`sequence_state_path`. Use a stable, local path on persistent storage and give
only the agent service account access to its parent directory. Create the
directory before starting the service, for example:

```sh
install -d -m 0700 -o skid-monitor-agent -g skid-monitor-agent \
  /var/lib/skid-monitor-agent
```

The file stores the next `x-skid-sequence` value. Before the first network
attempt for a signal, the exporter advances that value using a same-directory
temporary file, file `fsync`, atomic rename, and directory `fsync`. On Unix the
state and companion lock files are restricted to mode `0600`. The lock is held
for the exporter's lifetime, so a second process or exporter configured with the
same state path fails during startup instead of risking duplicate sequences.
Corrupt, unwritable, or otherwise unusable state also fails exporter
initialization. Do not delete or copy an active sequence state file; give each
provider client identity exactly one state path.

Unauthenticated OTLP exporters retain their legacy in-memory sequence because
they are intended for trusted local deployments. Authenticated cloud exporters
never derive restart identity from the wall clock.

Transient gRPC failures are retried at most twice after the initial attempt with
a short exponential backoff. All attempts for one logical export reuse the same
`x-skid-sequence`, so the ingress can deduplicate a retry after a lost ACK. An
`Unauthenticated` response invalidates the cached token once and retries with a
fresh token; permission and request-validation failures are not retried.

This state file is an allocator, not a payload spool. A crash after allocation
may leave a harmless sequence gap, and an in-process bounded retry reuses its
allocated value. A crash after the server accepts a payload but before the agent
observes the ACK cannot replay that exact payload/sequence after restart. A
future durable payload spool is required for process-crash exactly-once
delivery.

## Required exporter delivery

Every exporter named in a signal pipeline is currently required. The agent
attempts all of them and aggregates their failures instead of stopping at the
first error. Until an explicit required/optional delivery policy is added, do
not list a diagnostic exporter in a production pipeline unless its failure
should fail delivery for that signal.

An OTLP receiver returns the upstream SDK a generic gRPC `Unavailable` status
when any required downstream exporter fails, avoiding a false ACK without
exposing downstream addresses or authentication details. Device-socket delivery
surfaces an I/O error. Database-log delivery restores its pre-poll tail
checkpoint after a downstream failure, so the same lines are read again on the
next poll. Native self-observation cannot signal an upstream producer and logs
the aggregate failure instead.

The optional `scope` field in `auth` is sent to the token endpoint when a
deployment uses explicit OAuth scopes:

```json
"auth": {
  "token_url": "https://id.example/oauth2/token",
  "client_id": "agent-production-01",
  "client_secret_env": "SKID_MONITOR_OIDC_CLIENT_SECRET",
  "sequence_state_path": "/var/lib/skid-monitor-agent/cloud.sequence",
  "scope": "monitor-ingest"
}
```
