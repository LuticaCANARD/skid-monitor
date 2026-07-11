# Cloud / Solo 실행 모델

이 문서는 `skid-monitor`의 두 실행 모델과 신뢰 경계를 정의한다. 여기서 **Solo**는 한 사용자가
자기 장비에서 native client와 agent를 함께 쓰는 경우만 뜻한다. 사내망에 설치했다는 이유만으로
인증을 생략하는 일반적인 on-premise 모드는 Solo로 간주하지 않는다.

## 실행 모델

| 항목 | Solo | Cloud |
|---|---|---|
| Agent signal ingress | native client 프로세스에 포함 | `skid-monitor-ingress` |
| Client query/live access | 같은 native client의 메모리/UI | `skid-monitor-client-server` |
| 저장 | 기존 client SQLite projection/alert 저장 | PostgreSQL event store/projection |
| 인증/ACL | 없음. 숫자형 loopback 주소만 허용 | OIDC JWT + tenant RLS |
| 대상 | 한 사용자, 한 장비 | 여러 사용자/tenant/agent |

Solo의 native frontend는 `spawn_solo_receiver_managed_with_notify`를 사용한다. 시작 설정과 runtime
listener 추가가 모두 `127.0.0.0/8` 또는 `::1`의 숫자형 주소인지 확인하며, wildcard, LAN 주소,
hostname은 거부한다. 기존 unrestricted receiver API는 호환성을 위해 남아 있지만 Solo frontend는
그 API를 사용하지 않는다.

브라우저는 로컬 TCP listener를 열 수 없으므로 브라우저 단독으로는 Solo ingress가 될 수 없다.
브라우저를 로컬에서 쓸 때는 Rust native companion이 loopback ingress를 소유하고 WebSocket으로
relay해야 한다. 원격 사용자와 여러 사용자가 접근하는 설치는 아래 Cloud 모델을 사용한다.

## Cloud 데이터 흐름

```text
agent -- OTLP/gRPC + agent JWT --> skid-monitor-ingress
                                      |
                                      | commit, then ACK
                                      v
                    PostgreSQL signal_events + projection
                                      |
                     cursor replay + LISTEN/NOTIFY wake-up
                                      v
user client -- user JWT --> skid-monitor-client-server -- REST/WebSocket --> UI
```

두 서버는 서로 다른 공개 포트와 OIDC audience를 사용한다.

- ingress는 `telemetry-ingest` 역할이 있는 **agent service-account token**만 받는다.
- client server는 `telemetry-read` 역할이 있는 **user token**만 받는다.
- agent 등록/활성화 및 tenant bootstrap은 `telemetry-admin` 역할을 추가로 요구한다.
- 두 token 모두 설정된 tenant JSON pointer에 UUID 문자열을 가져야 한다.
- ingress의 agent ID는 기본적으로 token의 `azp` 또는 `client_id`에서 가져오며 provider별 JSON
  pointer로 바꿀 수 있다. 요청 body가 agent/tenant를 자칭할 수 없다.

Cloud ingress는 각 OTLP 요청에 `x-skid-sequence` metadata를 요구한다. 저장 key는
`(tenant_id, agent_id, sequence)`이고 같은 payload 재전송은 기존 cursor를 반환한다. 같은 sequence에
다른 payload가 오면 거부한다. PostgreSQL transaction 안에서 event, tenant projection과 commit-time
notification을 함께 기록하고 commit이 끝난 뒤에만 OTLP 성공 응답을 반환한다.
Agent는 이 값을 재시작 후에도 재사용하지 않도록 `sequence_state_path`에 다음 값을 먼저 원자적으로
저장한 뒤 송신한다. 이 파일은 payload spool이 아니므로, 프로세스가 ACK 경계에서 종료된 경우의
exactly-once 재전송까지 보장하지는 않는다.

## OIDC account provider

서버의 서명 검증은 provider 중립적이다. HTTPS OIDC discovery, JWKS, 정확한 issuer/audience,
`RS256`, `sub`/`iat`/`exp`와 최대 24시간 token 수명을 확인한 뒤 provider claim을 공통 principal로
정규화한다. 현재 role claim profile은 다음 두 가지다.

Provider별 pointer 예시와 Keycloak 전환 절차는
[OIDC account provider adapter](oidc-account-providers.md)에 정리한다.

| profile | role source | 용도 |
|---|---|---|
| `keycloak` (기본값) | `resource_access.<audience>.roles` | 기존 Keycloak 배포 및 설정 호환 |
| `generic` | `SKID_MONITOR_OIDC_ROLES_POINTER`의 문자열 배열 또는 공백 구분 문자열 | Authentik, Auth0, Okta, Entra ID, Cognito, Zitadel, Dex 등 |

공통 설정은 다음과 같다.

```sh
export SKID_MONITOR_OIDC_ISSUER='https://id.example.com/issuer'
export SKID_MONITOR_OIDC_CLAIMS_PROFILE='generic'
export SKID_MONITOR_OIDC_TENANT_POINTER='/organization/id'
export SKID_MONITOR_OIDC_ROLES_POINTER='/roles'
export SKID_MONITOR_OIDC_AGENT_ID_POINTER='/client_id' # 선택; 기본 azp/client_id
```

pointer는 RFC 6901 JSON pointer다. URL 형태의 custom claim
`https://monitor.example/roles`는 `/https:~1~1monitor.example~1roles`로 쓴다. user token에는 agent
identity가 없어도 되지만 ingress token에는 유효한 agent identity가 필요하다. 내부 authorization은
provider와 무관하게 `telemetry-ingest`, `telemetry-read`, `telemetry-admin` 역할만 사용한다.

Discovery가 돌려준 JWKS URL은 기본적으로 issuer와 같은 HTTPS origin이어야 한다. provider가 별도
origin을 사용하면 그 origin만 `SKID_MONITOR_OIDC_JWKS_ORIGIN=https://keys.example.com`으로 명시한다.
redirect, URL credential과 fragment는 허용하지 않는다.

### Keycloak profile

한 realm 안에 최소 두 resource client를 별도로 만든다.

1. `skid-monitor-ingress`
   - confidential client, service account 활성화
   - client role `telemetry-ingest`
   - 각 agent는 별도 client/service account를 쓰고 ingress audience를 token에 포함
2. `skid-monitor-client`
   - browser/native 로그인용 client와 별도 resource audience
   - client role `telemetry-read`, `telemetry-admin`

두 resource client의 `resource_access.<audience>.roles`에 해당 역할이 나타나도록 mapper를 설정한다.
realm role이나 다른 audience의 동명 role은 인정되지 않는다. 사용자/agent의 조직 UUID는 기본
`/tenant_id` pointer가 읽을 수 있게
`tenant_id` access-token claim으로 매핑한다. 기존 `SKID_MONITOR_KEYCLOAK_ISSUER`와
`SKID_MONITOR_TENANT_CLAIM`은 migration 기간의 fallback alias로만 유지한다.

## PostgreSQL

두 cloud 서버는 각각 이름이 같은 `SKID_MONITOR_DATABASE_URL` 환경변수를 사용하지만 Deployment별
Secret/login은 ingress와 client 최소 권한에 맞게 분리할 수 있다. schema 변경은 배포 전에 별도
migration job과 별도 DB role로 실행한다.
PostgreSQL 컴포넌트, role/grant, schema 계약과 현행 정지 배포 및 향후 online migration 절차는
[PostgreSQL 컴포넌트와 운영 migration](postgresql-components-and-migrations.md)에 상세히 정의한다.

```sh
export SKID_MONITOR_MIGRATION_DATABASE_URL='postgresql://skid_migrator:...@postgres/skid_monitor?sslmode=verify-full'
cargo run -p skid-monitor-server --bin skid-monitor-migrate
```

ingress와 client-access runtime은 migration을 실행하는 설정을 제공하지 않는다. 개발 환경도 먼저
`skid-monitor-migrate`를 실행해 runtime credential에 DDL 권한이 섞이지 않게 한다. migration은 다음
tenant-scoped table을 만든다.

- `tenants`, `agents`
- append-only `signal_events`
- 현재 `signal_projection`
- `audit_events`
- 1회용 browser 인증용 `stream_tickets`

모든 tenant table에는 PostgreSQL RLS와 `FORCE ROW LEVEL SECURITY`가 적용된다. 각 transaction은
JWT에서 검증된 UUID를 `app.tenant_id` local setting으로 먼저 넣는다. runtime은 직접 login으로
연결하고 `BYPASSRLS`, superuser, `CREATEDB`, `CREATEROLE`, `REPLICATION`이나
그 특권 role membership을 주지 않는다. 시작 시에는 `session_user=current_user`와 함께 `public`
schema CREATE 권한 및 application relation owner-role membership도 검사해 거부한다. migration/object-owner
credential이 schema object를 소유하고 runtime role에는 schema `USAGE`, tenant table의
필요한 `SELECT/INSERT/UPDATE/DELETE`, identity sequence 권한과 readiness용
`GRANT SELECT ON public._sqlx_migrations TO <runtime-role>`만 부여한다. readiness는 embedded migration의
version/checksum, 필수 column, validated constraint, required index의 valid/ready 상태,
RLS/FORCE RLS와 tenant isolation policy 전체 집합이 정확한지 검사한다.
모든 PostgreSQL pool connection은 URL의 `search_path` 옵션과 무관하게 `pg_catalog, public`으로 다시
고정하므로 migration과 runtime query가 readiness에서 검증한 schema 밖의 동명 relation을 사용하지 않는다.
`tenants.enabled=false`이면 신규 ingest, replay, agent 관리와 stream ticket 작업을 모두 거부한다.
tenant당 agent identity는 1,000개로 제한하고, 저장 전 canonical JSON 왕복 검증과 16 MiB hard cap을
적용해 replay할 수 없는 non-finite 수치나 과대 payload가 commit되는 것을 막는다.

## 서버 실행 설정

공통 설정:

```sh
export SKID_MONITOR_DATABASE_URL='postgresql://skid_monitor:...@postgres/skid_monitor?sslmode=verify-full'
export SKID_MONITOR_DATABASE_MAX_CONNECTIONS=12
export SKID_MONITOR_OIDC_ISSUER='https://id.example.com/realms/skid-monitor'
export SKID_MONITOR_OIDC_CLAIMS_PROFILE='keycloak'
export SKID_MONITOR_OIDC_TENANT_POINTER='/tenant_id'
```

Cloud DB 연결은 기본적으로 `sslmode=verify-full`을 강제한다. service mesh나 private tunnel이
PostgreSQL TLS까지 신뢰 구간에서 대신 종료하는 배포만 URL에 `sslmode=disable`을 명시하고
`SKID_MONITOR_DATABASE_TLS_TERMINATED=true`를 함께 설정할 수 있다. `prefer`, `require`, URL 기본값은
인증서/hostname 검증이 보장되지 않으므로 cloud server가 거부한다.

Ingress:

```sh
export SKID_MONITOR_INGRESS_ADDR='0.0.0.0:4317'
export SKID_MONITOR_INGRESS_AUDIENCE='skid-monitor-ingress'
export SKID_MONITOR_INGRESS_ROLE='telemetry-ingest'
export SKID_MONITOR_INGRESS_CONCURRENCY_PER_CONNECTION=64
export SKID_MONITOR_INGRESS_GLOBAL_REQUEST_CONCURRENCY=16
export SKID_MONITOR_INGRESS_TLS_CERT='/run/secrets/ingress.crt'
export SKID_MONITOR_INGRESS_TLS_KEY='/run/secrets/ingress.key'
cargo run -p skid-monitor-server --bin skid-monitor-ingress
```

`SKID_MONITOR_INGRESS_CONCURRENCY_PER_CONNECTION`은 한 HTTP/2 연결 안의 요청 수를 제한한다.
여러 연결이 이를 우회하지 못하도록 `SKID_MONITOR_INGRESS_GLOBAL_REQUEST_CONCURRENCY`가 metrics,
traces, logs 전체에서 protobuf decode 전 동시 요청 수를 하나의 semaphore로 제한한다. 전역 기본값은
16이고 입력값은 최대 128로 제한된다. 이 값은 PostgreSQL pool 크기와 배포 replica 수를 함께 고려해
조정한다.

Client access server:

```sh
export SKID_MONITOR_CLIENT_SERVER_ADDR='0.0.0.0:8080'
export SKID_MONITOR_CLIENT_AUDIENCE='skid-monitor-client'
export SKID_MONITOR_CLIENT_READ_ROLE='telemetry-read'
export SKID_MONITOR_CLIENT_ADMIN_ROLE='telemetry-admin'
export SKID_MONITOR_STREAM_BATCH_BYTES=16777216
export SKID_MONITOR_CLIENT_REQUEST_CONCURRENCY=256
export SKID_MONITOR_CLIENT_REPLAY_CONCURRENCY=4
export SKID_MONITOR_CLIENT_MAX_STREAM_CONNECTIONS=1024
export SKID_MONITOR_CLIENT_TLS_CERT='/run/secrets/client.crt'
export SKID_MONITOR_CLIENT_TLS_KEY='/run/secrets/client.key'
cargo run -p skid-monitor-server --bin skid-monitor-client-server
```

TLS를 service mesh/ingress controller가 종료하는 경우에만 각각
`SKID_MONITOR_INGRESS_TLS_TERMINATED=true`,
`SKID_MONITOR_CLIENT_TLS_TERMINATED=true`를 명시한다. certificate/key도 없고 이 opt-in도 없으면
서버는 시작하지 않는다. 평문 listener는 신뢰된 private network 안에만 둔다.

## Agent cloud exporter

Agent secret은 JSON 파일에 쓰지 않고 환경변수로 주입한다.

```json
{
  "exporters": {
    "cloud": {
      "type": "otlp",
      "endpoint": "https://signals.example.com:4317",
      "auth": {
        "token_url": "https://id.example.com/oauth2/token",
        "client_id": "agent-node-a",
        "client_secret_env": "SKID_MONITOR_OIDC_CLIENT_SECRET",
        "sequence_state_path": "/var/lib/skid-monitor-agent/cloud.sequence"
      }
    }
  },
  "pipelines": {
    "metrics": { "exporters": ["cloud"] },
    "traces": { "exporters": ["cloud"] },
    "logs": { "exporters": ["cloud"] }
  }
}
```

OAuth client-credentials와 authenticated OTLP endpoint는 모두 HTTPS여야 한다. agent는 access token을
만료 전에 갱신하고, 모든 signal 종류에 Bearer token과 sequence metadata를 붙인다. authenticated
exporter는 `sequence_state_path`가 반드시 필요하다. state의 부모 디렉터리는 서비스 계정만 쓸 수 있는
persistent volume으로 미리 생성해야 한다. 동일 경로를 쓰는 두 프로세스는 companion lock에 의해 시작이
거부된다. agent는 network 전송 전에 다음 sequence를 임시 파일, `fsync`, atomic rename으로 기록하므로
정상적인 process/container 재시작이나 wall-clock rollback으로 이미 사용한 sequence를 다시 쓰지 않는다.
기존 auth 없는 OTLP exporter 설정은 로컬/개발 호환성을 위해 계속 사용할 수 있다.

## Client API

| Method | Path | 역할 | 용도 |
|---|---|---|---|
| `GET` | `/health/live` | 없음 | process liveness |
| `GET` | `/health/ready` | 없음 | PostgreSQL readiness |
| `GET` | `/v1/signals?after=N&limit=N` | `telemetry-read` | durable cursor replay |
| `GET` | `/v1/projection` | `telemetry-read` | tenant summary projection |
| `POST` | `/v1/stream-tickets` | `telemetry-read` | 30초 유효한 1회용 browser stream ticket |
| `GET` | `/v1/stream?after=N` | `telemetry-read` | live WebSocket + missed-event replay |
| `PUT` | `/v1/tenant` | `telemetry-admin` | token의 현재 tenant 최초 생성/수정 |
| `GET` | `/v1/agents` | `telemetry-read` | 등록 agent 목록 |
| `POST` | `/v1/agents` | `telemetry-admin` | agent 등록/재활성화 |
| `PATCH` | `/v1/agents/{agent_id}` | `telemetry-admin` | agent 활성화/비활성화 |

일반 HTTP/native WebSocket client는 `Authorization: Bearer ...`를 보낸다. 브라우저 WebSocket은
Authorization header를 설정할 수 없으므로 먼저 Bearer token으로 `/v1/stream-tickets`를 호출한 뒤,
반환된 짧은 수명의 1회용 값을 `/v1/stream?ticket=<tenant_uuid.ticket_uuid>&after=N`에 사용한다.
장기 access token은 URL이나 `Sec-WebSocket-Protocol`에 넣지 않는다. ticket은 PostgreSQL에서 원자적으로
한 번만 소비되고 30초 뒤 만료되므로 여러 client-server replica에서도 같은 규칙을 유지한다. ticket은
원래 OIDC token의 `exp`도 보존하며, 연결된 WebSocket은 그 시각에 종료되어 재인증을 요구한다.
application request span은 path만 기록하지만, 외부 load balancer/reverse proxy도 `/v1/stream`의 query
string을 access log에서 제거하거나 redaction해 30초 ticket이 로그에 남지 않게 한다.

WebSocket notification은 전달 보장이 아니라 조회 wake-up 용도다. 연결이 끊기거나 notification을
놓쳐도 client가 마지막 cursor로 PostgreSQL을 다시 조회하므로 event를 복구할 수 있다.
각 WebSocket text message는 `{ "cursor": ..., "envelope": ... }` 형태의 `SignalRecord`다.
browser frontend는 `?client_api=https://monitor.example/client`로 cloud API mode를 선택한다. `http://`는
개발용 숫자형 loopback 주소에만 허용되고 hostname이나 non-loopback 평문 endpoint는 token을 읽기 전에
거부한다. 또한 adapter는 frontend와 cloud API의 origin이 같은지 token을 읽기 전에 확인한다. 분리된
client-access server는 same-origin reverse proxy 경로 아래에 둔다. OIDC Authorization Code + PKCE
login/refresh를 소유한 host shell은 access token을 `sessionStorage`의
`skid-monitor.oidc.access_token` key에 넣는다. 기존
`skid-monitor.keycloak.access_token`은 migration fallback으로 읽기만 한다. adapter는 연결 시마다
최신 token으로 stream ticket을 발급받고 `<tenant_uuid>.<ticket_uuid>` 전체 형식을 검증한다. ticket
원문은 저장하지 않고 tenant UUID만 사용하여, 수신 큐에 전달한 cursor를 API endpoint+tenant별
`localStorage` key에 기록해 끊김이나 token 만료 뒤 `after`로 replay한다. 수신 `SignalRecord`의 tenant도
인증된 stream tenant와 일치해야 한다. edge/alert browser state도 같은
endpoint+tenant namespace를 사용한다. 인증 tenant가 확인되기 전에는 browser state를 복원하거나 쓰지
않고, tenant가 바뀌면 in-memory signal state를 비운 뒤 새 namespace만 복원한다. 기존 endpoint-only
cursor와 unscoped browser state는 cloud namespace로 승계하지 않는다. access token은 URL이나 Rust 장기
상태에 저장하지 않는다. 현재 dashboard는 한 security scope만 보유하므로 cloud API endpoint는 하나만
연결할 수 있고 raw bridge와 동시에 사용할 수 없다. 기존 raw bridge는 `?ingress=wss://...`로 계속 사용할
수 있지만 이 mode에는 ticket/replay 보장이 적용되지 않는다.

Replay는 row 개수뿐 아니라 `SKID_MONITOR_STREAM_BATCH_BYTES` byte budget(기본 16 MiB, 최대 64 MiB)을
적용한다. 한 개의 큰 signal은 forward progress를 위해 반환할 수 있지만, 여러 최대 크기 signal을 한 번에
`fetch_all`하지 않는다. REST replay와 WebSocket batch load/send는 별도의 전역 replay semaphore를
공유한다. `SKID_MONITOR_CLIENT_REPLAY_CONCURRENCY`의 기본값은 4이고 hard cap은 16이다. REST JSON은
64 KiB 이하 chunk로 전송하며 response body가 완료되거나 취소될 때까지 해당 permit을 유지하므로, 느린
client마다 큰 materialized response가 동시에 누적되지 않는다. 이 제한은
`SKID_MONITOR_CLIENT_MAX_STREAM_CONNECTIONS`가 제어하는 WebSocket 연결 수와 별개다. WebSocket은 각
batch를 조회하고 모두 전송할 때까지 replay permit을 유지하며 느린 client send timeout도 적용한다. 외부
reverse proxy에서도 IP/tenant별 request rate, connection rate와 최대 body 크기를 별도로 제한해야 한다.

## 운영 데이터 보존 경계

현재 migration은 `signal_events`와 `audit_events`를 자동 삭제하지 않는다. agent cardinality 1,000개 제한은
projection/list 메모리를 제한하지만 tenant의 장기 event 발생량이나 PostgreSQL disk 사용량까지 제한하지는
않는다. 운영 전 managed PostgreSQL의 backup/PITR, disk 경보와 함께 별도 privileged maintenance job 또는
time partition 정책으로 보존 기간과 tenant별 용량을 정해야 한다. 이 작업에 runtime RLS role을 승격하지
말고 별도 credential과 감사 경계를 사용한다. event를 만료해도 `signal_projection`은 누적 projection으로
남으므로 보존 창 통계가 필요하면 별도 windowed projection을 추가해야 한다.

## 현재 내구성 경계

Cloud exporter는 한 논리적 OTLP export 안의 일시 오류를 최대 3회 재시도하며 같은 sequence를 재사용한다.
Agent의 local OTLP receiver는 required downstream export가 최종 실패하면 성공 ACK를 반환하지 않고,
database-log tail도 offset을 되돌려 다음 poll에서 다시 시도한다. authenticated exporter의 sequence는
위 state file에 crash-safe하게 먼저 기록하지만, 미전송 payload와 exporter별 delivery 결과를 local disk
spool에 영속화하지는 않는다. state volume을 삭제/rollback하거나 같은 OIDC agent identity를 서로 다른
state file로 동시 실행하는 경우, 또는 여러 required exporter 중 일부만 성공한 경우까지 exactly-once를
보장하지 않는다. 이 조건이 필요한 배포는 agent credential, process, persistent sequence file을 1:1로
유지하고 agent별 단일 cloud exporter를 사용하며, 후속 durable payload spool 전까지 upstream SDK retry
정책을 함께 둔다.
