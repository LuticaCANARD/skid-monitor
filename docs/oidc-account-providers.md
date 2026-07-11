# OIDC account provider adapter

Cloud mode의 account provider는 Keycloak으로 고정되지 않는다. Rust server는 OIDC access token을
검증한 뒤 provider별 claim 형태를 다음 내부 principal로 정규화한다.

```text
subject + tenant UUID + roles + optional agent identity + token expiry
```

PostgreSQL RLS, stream ticket, audit와 `telemetry-ingest`/`telemetry-read`/`telemetry-admin` authorization은
provider를 알지 못한다. 따라서 provider 교체는 database migration이 아니라 OIDC 설정과 token mapper
migration이다.

## 공통 검증 계약

- issuer의 HTTPS OIDC discovery 문서를 사용한다.
- discovery의 issuer가 설정값과 정확히 같아야 한다.
- JWKS는 issuer와 같은 HTTPS origin만 기본 허용한다. 다른 origin은
  `SKID_MONITOR_OIDC_JWKS_ORIGIN`으로 하나만 명시한다.
- redirect, URL credential과 fragment를 허용하지 않는다.
- 현재 access-token 서명 알고리즘은 `RS256`만 허용한다.
- `iss`, `aud`, `sub`, `iat`, `exp`가 필요하며 token lifetime은 최대 24시간이다.
- tenant claim은 UUID 문자열이어야 한다.
- role은 최대 256개, 각 256 bytes 이하의 문자열이어야 한다.
- user token에는 agent identity가 없어도 된다. ingress token에는 필수다.

## 설정

모든 provider가 사용하는 기본 설정:

```sh
export SKID_MONITOR_OIDC_ISSUER='https://accounts.example.com/issuer'
export SKID_MONITOR_OIDC_TENANT_POINTER='/tenant_id'
export SKID_MONITOR_INGRESS_AUDIENCE='skid-monitor-ingress'
export SKID_MONITOR_CLIENT_AUDIENCE='skid-monitor-client'
```

`SKID_MONITOR_OIDC_TENANT_POINTER`, `SKID_MONITOR_OIDC_ROLES_POINTER`,
`SKID_MONITOR_OIDC_AGENT_ID_POINTER`는 RFC 6901 JSON pointer다. `/organization/id`는 다음 token을 읽는다.

```json
{
  "organization": { "id": "4d6f5ef3-f18d-4930-a3c8-13f013c9a004" }
}
```

claim 이름 자체에 `/` 또는 `~`가 있으면 각각 `~1`, `~0`으로 escape한다.

## Keycloak profile

기존 동작을 그대로 유지하는 기본 profile이다.

```sh
export SKID_MONITOR_OIDC_CLAIMS_PROFILE='keycloak'
```

role은 다음 위치에서 audience별로 읽는다.

```json
{
  "resource_access": {
    "skid-monitor-client": {
      "roles": ["telemetry-read", "telemetry-admin"]
    }
  }
}
```

agent identity는 명시적 pointer가 없으면 `azp`, 그다음 `client_id`를 사용한다. 기존
`SKID_MONITOR_KEYCLOAK_ISSUER`와 top-level claim 이름을 받던 `SKID_MONITOR_TENANT_CLAIM`은 호환
fallback이다. 신규 배포에서는 OIDC 이름을 사용한다.

## Generic profile

OIDC provider가 role을 flat/nested/custom claim으로 발급할 때 사용한다.

```sh
export SKID_MONITOR_OIDC_CLAIMS_PROFILE='generic'
export SKID_MONITOR_OIDC_ROLES_POINTER='/roles'
export SKID_MONITOR_OIDC_AGENT_ID_POINTER='/client_id'
```

role claim은 문자열 배열 또는 OAuth scope 형태의 공백 구분 문자열을 받을 수 있다.

```json
{ "roles": ["telemetry-read", "telemetry-admin"] }
```

```json
{ "scope": "openid telemetry-read profile" }
```

두 번째 형태는 `SKID_MONITOR_OIDC_ROLES_POINTER=/scope`로 설정한다. URL namespace custom claim 예시는
다음과 같다.

```json
{
  "https://monitor.example/roles": ["telemetry-read"]
}
```

이때 pointer는 `/https:~1~1monitor.example~1roles`이다.

## Provider 적용 기준

Auth0, Okta, Authentik, Azure Entra ID, AWS Cognito, Zitadel, Dex 등 OIDC provider는 generic profile로
연결할 수 있다. 실제 claim 이름은 tenant/authorization-server 설정에 따라 다르므로 provider 이름을
코드에 넣는 대신 발급된 staging access token의 claim 계약을 확인해 pointer를 고정한다.

일반적인 출발점은 다음과 같지만 배포 token을 기준으로 검증해야 한다.

| token 형태 | 설정 후보 |
|---|---|
| `roles: [...]` | roles pointer `/roles` |
| `groups: [...]` | roles pointer `/groups` |
| `scope: "..."` | roles pointer `/scope` |
| `tid` tenant UUID | tenant pointer `/tid` |
| namespaced organization claim | escaped custom pointer |
| `appid`, `client_id`, `azp` agent identity | 해당 agent-id pointer |

Provider group 전체를 곧바로 admin role로 신뢰하지 않는다. token mapper에서 필요한 세 내부 역할만
발급하거나 dedicated custom claim으로 줄인다. ingress와 client audience도 분리한다.

## Browser와 agent

Browser host shell은 provider SDK에 종속될 수 있지만 Rust/WASM adapter는 종속되지 않는다. shell은
Authorization Code + PKCE로 받은 최신 access token을 다음 session-only key에 넣는다.

```text
skid-monitor.oidc.access_token
```

기존 `skid-monitor.keycloak.access_token`은 migration fallback이며 신규 shell은 사용하지 않는다.
logout 시 두 key를 모두 지운다. token을 URL이나 `localStorage`에 넣지 않는다.

Agent exporter는 표준 OAuth 2.0 client credentials를 사용한다. `token_url`, `client_id`, secret 환경변수와
필요한 `scope`를 provider에 맞게 설정한다. ingress audience, tenant, role, agent identity가 최종 access
token에 포함되어야 하며 agent JSON이 이 값을 자칭할 수는 없다.

## Keycloak에서 다른 provider로 migration

1. 신규 provider에 ingress/client audience와 세 내부 role을 만든다.
2. tenant UUID와 agent identity claim mapper를 구성한다.
3. staging token으로 issuer, audience, pointer, role, agent ID를 확인한다.
4. server에 `SKID_MONITOR_OIDC_*` 설정을 배포하고 canary에서 ingest/read/admin/RLS를 검증한다.
5. browser shell을 `skid-monitor.oidc.access_token` key로 전환한다.
6. agent credential을 provider별로 순차 교체한다. agent identity와 durable sequence file의 1:1 관계를
   유지한다.
7. 모든 client/agent가 전환된 뒤 legacy Keycloak 환경변수와 session key를 제거한다.

Database schema migration은 없다. provider 전환 중에도 같은 tenant UUID를 유지해야 기존 PostgreSQL
tenant row와 RLS scope가 이어진다.
