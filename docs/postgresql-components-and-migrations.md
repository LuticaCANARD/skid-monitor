# PostgreSQL 컴포넌트와 운영 migration

이 문서는 Cloud mode에서 사용하는 PostgreSQL의 서비스 경계, 데이터 모델, 권한, 운영 책임과
schema migration 절차를 정의한다. Solo mode는 PostgreSQL을 사용하지 않으며 이 문서의 대상이 아니다.

문서의 상태 표기는 다음과 같다.

- **현재**: 저장소에 실행 코드, SQL 또는 검증 로직이 존재한다.
- **운영 구성**: 애플리케이션 코드 밖에서 운영자가 반드시 준비해야 한다.
- **계획**: production 운영을 위해 추가해야 하지만 아직 저장소에 구현되지 않았다.

## 서비스 경계

```text
                                       Keycloak
                               JWT issuer / JWKS / roles
                                  /                 \
                                 v                   v
agent -- OTLP/gRPC --> skid-monitor-ingress    skid-monitor-client-server <-- REST/WS -- client
                              |                    |       |
                              | runtime pool       | pool  | dedicated LISTEN session
                              +--------------------+-------+
                                                   |
                                      managed PostgreSQL primary
                                      public schema + tenant RLS
                                                   |
                              +--------------------+--------------------+
                              |                    |                    |
                    skid-monitor-migrate   maintenance job       managed backup/PITR
                      privileged pool        (planned)           and HA (operations)
```

PostgreSQL은 인증 서버가 아니다. Keycloak이 사용자와 agent를 인증하고, Rust 서버가 검증된
`tenant_id` UUID를 transaction-local `app.tenant_id`에 넣는다. PostgreSQL RLS는 애플리케이션의
tenant 조건에 더해 적용되는 최종 데이터 격리선이다.

### 컴포넌트 책임

| 컴포넌트 | 상태 | PostgreSQL 책임 |
|---|---|---|
| Managed PostgreSQL service | 운영 구성 | primary, multi-AZ/replication, storage encryption, patching, backup/PITR, 장애조치 |
| `skid-monitor-ingress` | 현재 | tenant/agent 확인, event append, projection/`last_seen_at` 갱신, commit 시 notify, commit 후 OTLP ACK |
| `skid-monitor-client-server` | 현재 | tenant/agent 관리, replay/projection 조회, audit 기록, 일회용 stream ticket 관리 |
| client notification listener | 현재 | client server replica마다 별도 PostgreSQL session으로 `LISTEN skid_monitor_signal_events` 수행 |
| `skid-monitor-migrate` | 현재 | embedded SQLx migration을 production runtime과 분리된 credential로 적용 |
| retention/maintenance job | 계획 | event/audit/ticket 보존, backfill, vacuum 영향 관리; runtime credential을 사용하지 않음 |
| backup/PITR와 restore 검증 | 운영 구성 | managed service 정책과 별도 복구 훈련으로 수행; application pod가 backup을 만들지 않음 |

Managed service의 control plane, WAL 보관, replica 승격, snapshot과 암호화 키는 데이터베이스
운영 영역이다. 애플리케이션은 schema와 query, tenant 격리, migration artifact를 소유한다. Keycloak과
PostgreSQL은 서로 독립된 장애 도메인으로 두며 데이터베이스를 public network에 직접 노출하지 않는다.

## 연결 구조와 용량

### 현재 연결

- ingress와 client server는 각각 자기 Deployment에 주입된 `SKID_MONITOR_DATABASE_URL`로 독립
  `sqlx::PgPool`을 만든다. 환경변수 이름은 같지만 Secret 값은 서로 다른 least-privilege login으로
  구성할 수 있다.
- 두 runtime pool의 기본 최대 연결 수는 replica당 12개이다.
- client server replica는 pool과 별개로 알림 전용 `PgListener` session 하나를 유지한다. 연결이
  끊어지면 1초 뒤 재연결한다.
- migrator는 `SKID_MONITOR_MIGRATION_DATABASE_URL`과 기본 최대 연결 2개를 사용한다.
- pool은 연결마다 `search_path`를 `pg_catalog, public`으로 강제한다. URL에 주입된 `search_path`는
  신뢰하지 않는다. 알림 전용 session은 application relation을 조회하지 않고 `LISTEN`만 수행한다.
- pool의 acquire timeout은 5초, idle timeout은 10분, max lifetime은 30분이다.

배포 전 다음 상한으로 managed service의 `max_connections`를 산정한다.

```text
필요 연결 수 = ingress_replicas * ingress_pool_max
             + client_replicas * (client_pool_max + 1 LISTEN session)
             + 동시에 실행 가능한 migration/maintenance 연결
             + 운영/모니터링/비상 접속 reserve
```

상한을 그대로 예약하는 대신 실제 동시 query와 headroom을 관찰해 조정하되, replica 증가가 DB 연결
한도를 넘지 않도록 배포 admission 또는 경보를 둔다. 외부 pooler는 **계획**이다. transaction pooling을
도입한다면 transaction-local tenant context와 transaction advisory lock을 검증하고, `LISTEN` 연결은
반드시 session mode 또는 PostgreSQL 직결로 분리해야 한다.

### TLS

**현재** runtime과 migrator URL은 다음 중 하나만 허용한다.

1. `sslmode=verify-full`: 인증서 chain과 hostname을 PostgreSQL client가 확인한다.
2. `sslmode=disable`과 `SKID_MONITOR_DATABASE_TLS_TERMINATED=true`: 신뢰된 private tunnel/service mesh가
   DB 구간 TLS를 종료하는 배포에서만 명시적으로 허용한다. migrator는 대응하는
   `SKID_MONITOR_MIGRATION_DATABASE_TLS_TERMINATED=true`를 사용한다.

`prefer`, `require`, URL 기본값은 거부한다. DB Secret은 ingress, client, migrator, maintenance,
backup 주체별로 분리하고 로그나 manifest 평문에 넣지 않는다.

## 현재 schema

현재 migration artifact는
`skid-monitor-server/migrations/0001_cloud_signal_store.sql` 하나이며 `public` schema에 적용된다.
SQLx 이력 table 이름은 `public._sqlx_migrations`로 고정되어 있다. 별도 PostgreSQL extension은
요구하지 않는다.

### relation

| relation | 역할과 주요 key | 보존 특성 |
|---|---|---|
| `_sqlx_migrations` | version, description, 적용 시각, success, checksum, execution time | SQLx 관리; tenant RLS 대상 아님 |
| `tenants` | `id UUID` PK, unique `slug`, 표시명, `enabled` | tenant root; `enabled=false`이면 ingest/read/admin/ticket 작업 거부 |
| `agents` | `(tenant_id, agent_id)` PK, 표시명, 활성 상태, 등록/최근 수신 시각 | tenant당 새 identity 최대 1,000개를 Rust transaction에서 제한 |
| `signal_events` | identity `cursor` PK, tenant/event/agent/sequence, kind, JSONB payload, payload byte 수 | append event store; 현재 자동 삭제/partition 없음 |
| `signal_projection` | `tenant_id` PK, `last_cursor`, 누적 projection JSONB | event append transaction에서 동기 갱신 |
| `audit_events` | identity `id` PK, actor/action/target/details | client admin mutation 감사; 현재 자동 삭제 없음 |
| `stream_tickets` | UUID ticket PK, tenant/subject, auth/expiry/consume 시각 | 1회용; 생성 시 같은 tenant의 만료/소비 ticket만 opportunistic 삭제 |

`signal_events.sequence`와 `received_at_unix_nano`는 전체 `u64` 범위를 보존하기 위해
`NUMERIC(20,0)`이다. payload는 canonical JSON object로 검증되며 저장 크기는 16 MiB 이하이다.
`payload_bytes`는 replay byte budget 계산에서 큰 JSONB를 미리 detoast하지 않기 위한 metadata이다.

### key, constraint와 index

명시적으로 생성되는 index는 다음과 같다.

- `signal_events_tenant_cursor_idx (tenant_id, cursor)`: cursor replay.
- `signal_events_tenant_committed_at_idx (tenant_id, committed_at DESC)`: 시간 기준 운영 조회/향후 보존.
- `audit_events_tenant_occurred_at_idx (tenant_id, occurred_at DESC, id DESC)`: tenant 감사 조회.
- `stream_tickets_cleanup_idx (tenant_id, expires_at, consumed_at)`: ticket 정리.

PK와 unique constraint도 PostgreSQL이 backing B-tree index를 만든다. 특히 다음 uniqueness가 ingest
idempotency와 충돌 검출의 일부이다.

- `signal_events(tenant_id, event_id)` unique.
- `signal_events(tenant_id, agent_id, sequence)` unique. 같은 signal 재시도는 기존 cursor를 반환하고,
  같은 sequence의 다른 kind/payload는 Rust store가 conflict로 거부한다.
- `tenants(slug)` unique.

FK는 tenant 삭제를 `agents`, projection, audit, ticket과 event까지 cascade한다. 다만 event에서 agent로의
FK는 agent 삭제에 `RESTRICT`이므로 현재 API는 agent를 삭제하지 않고 enable/disable한다. kind는
`metrics`, `traces`, `logs`만 허용하며 JSONB object, 값 범위, 비어 있지 않은 식별자와 ticket 시간
관계를 CHECK constraint로 방어한다.

### RLS와 transaction 경계

`tenants`, `agents`, `signal_events`, `signal_projection`, `audit_events`, `stream_tickets` 모두
`ENABLE ROW LEVEL SECURITY`와 `FORCE ROW LEVEL SECURITY`가 적용된다. 현재 policy는 relation별
`<relation>_tenant_isolation` 하나이며 permissive `ALL`, 대상은 `PUBLIC`이다. table grant가 첫 번째
권한 경계이고 다음 식의 `USING`과 `WITH CHECK`가 두 번째 경계이다.

```sql
tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid
```

`tenants`만 `tenant_id` 대신 `id`를 비교한다. Rust store는 tenant 작업마다 transaction을 시작하고
`set_config('app.tenant_id', '<validated UUID>', true)`를 실행한다. 세 번째 인자 `true` 때문에 값은
transaction-local이고 pool에 반환된 연결로 누출되지 않는다. tenant context가 없으면 policy 식은
`NULL`이 되어 row를 노출하지 않는다.

event append는 한 transaction에서 다음 순서로 수행한다.

1. tenant context와 tenant enabled 상태를 확인한다.
2. tenant UUID에서 만든 key로 `pg_advisory_xact_lock`을 획득한다.
3. 등록되고 활성화된 agent인지 확인한다.
4. event를 insert하거나 동일 sequence 재시도를 확인한다.
5. projection과 agent `last_seen_at`을 갱신한다.
6. `pg_notify`를 호출하고 commit한다.
7. commit 성공 뒤에만 OTLP ACK를 반환한다.

tenant별 advisory lock은 cursor N+1이 N보다 먼저 commit되어 client가 N을 영구히 건너뛰는 것을 막는다.
이 lock의 대기 시간과 tenant hot spot은 운영 지표로 관찰해야 한다.

## LISTEN/NOTIFY

현재 channel은 `skid_monitor_signal_events`이고 payload는 다음 작은 JSON이다.

```json
{"tenant_id":"<uuid>","cursor":123}
```

알림은 event/projection과 같은 transaction 안에서 생성되므로 commit되지 않은 event를 깨우지 않는다.
`NOTIFY`는 durable queue나 event 본문 운반 수단이 아니다. client server는 알림을 tenant별 in-process
broadcast wake-up으로만 사용하고, 실제 데이터는 `signal_events`에서 cursor로 다시 읽는다. 알림 손실,
listener 재연결, 구독 경합을 보완하기 위해 WebSocket stream은 2초 fallback poll을 수행한다. 따라서
HA failover 중 notification이 사라져도 cursor replay가 정확성의 기준이다.

Replica가 많으면 각 replica가 같은 notification을 받고 자기 WebSocket만 깨운다. PostgreSQL notification
queue 사용량, listener 재연결 수와 fallback poll 증가를 관찰한다. 장기 fan-out 규모에서 PostgreSQL
notification이 병목이 되면 전용 broker 도입을 별도 설계하되, durable source of truth는 계속 event
cursor여야 한다.

## 데이터베이스 role

### 현재 강제되는 조건

role과 grant를 생성하는 SQL은 아직 저장소에 포함되어 있지 않다. 운영자가 managed PostgreSQL에서
provision해야 한다. runtime 시작 시 실제 login과 current role이 다르거나 현재 role 또는 전환 가능한
role에 `SUPERUSER`, `BYPASSRLS`, `CREATEDB`, `CREATEROLE`, `REPLICATION`이 있거나 `public` schema
`CREATE` 권한/application relation owner membership이 있으면 실패한다. 또한
`public._sqlx_migrations`를 읽을 수 있어야 readiness checksum 검증을 통과한다. runtime binary에는
migration opt-in이 없고 object owner와 runtime login은 분리해야 한다.

### 목표 role 모델

아래 role 분리는 **운영 구성/계획**이다. 실제 이름은 환경 prefix를 붙일 수 있지만 책임은 합치지 않는다.

| role | LOGIN | 권한과 금지사항 |
|---|---:|---|
| `skid_owner` | 아니오 | database application schema/object/policy의 안정적인 owner. application pod가 직접 사용하지 않음 |
| `skid_migrator` | 예 | 배포 Job만 사용. owner로 명시적으로 `SET ROLE`한 뒤 versioned DDL 수행. runtime Secret과 분리 |
| `skid_ingress_runtime` | 예 | tenant/agent 확인, signal insert/read-on-conflict, projection/agent update에 필요한 DML만 보유 |
| `skid_client_runtime` | 예 | tenant/agent admin, replay/projection, audit insert, stream-ticket DML에 필요한 권한만 보유 |
| `skid_maintenance` | 예 또는 NOLOGIN group | retention/backfill/reindex 전용. 일정 시간에만 credential 활성화; 작업별 최소 DML/DDL |
| `skid_backup` | 예 또는 provider identity | managed backup이면 SQL login을 만들지 않음. `pg_dump` 보조 백업 시에만 전 tenant read 권한을 제한적으로 부여 |

runtime 공통 조건은 직접 login, `NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS`이며
이 특권 role의 member도 아니어야 한다. `public` schema에는
`USAGE`만 주고 `CREATE`를 주지 않는다. PUBLIC의 schema create 권한도 회수한다. identity column을
insert하는 role에는 해당 identity sequence의 `USAGE`를 주고, 두 runtime 모두
`SELECT ON public._sqlx_migrations`를 가진다.

정확한 최소 table grant 목표는 다음과 같다.

| relation | ingress runtime | client runtime |
|---|---|---|
| `tenants` | `SELECT` | `SELECT, INSERT, UPDATE` |
| `agents` | `SELECT, UPDATE` | `SELECT, INSERT, UPDATE` |
| `signal_events` | `SELECT, INSERT` | `SELECT` |
| `signal_projection` | `SELECT, INSERT, UPDATE` | `SELECT` |
| `audit_events` | 없음 | `INSERT` |
| `stream_tickets` | 없음 | `SELECT, INSERT, UPDATE, DELETE` |

현재 RLS policy가 `PUBLIC` 대상이라는 사실을 table grant가 보완한다. policy 대상을 runtime group으로
좁히려면 readiness가 policy의 `PUBLIC` 대상을 정확히 검사하므로 먼저 Rust 검증 코드와 migration을
같은 호환성 계획으로 변경해야 한다.

`skid_owner` + `SET ROLE` 모델은 아직 migrator binary에 구현되지 않았다. 도입 전에는 migration
connection role이 직접 object owner여야 한다. 도입할 때 migrator는 migration 시작 전 owner role
전환을 검증하고, 생성된 모든 relation/sequence/function owner와 runtime grant를 postflight에서 확인해야
한다. 새 object의 grant를 우연한 default privilege에만 맡기지 말고 migration SQL에 명시한다.

Maintenance가 모든 tenant를 한 번에 처리해야 한다고 해서 runtime에 `BYPASSRLS`를 추가하면 안 된다.
가능하면 tenant별로 local context를 설정해 작업한다. cross-tenant retention이나 논리 backup 때문에
`BYPASSRLS`가 불가피하면 maintenance/backup 전용 role에만 부여하고, private network, 짧은 Secret TTL,
작업 시간 제한과 별도 audit를 적용한다. `FORCE ROW LEVEL SECURITY` 때문에 object owner도 RLS를 따르므로
owner credential만으로는 tenant context 없는 전체 backfill이 허용되지 않는다는 점을 전제로 한다.

## readiness와 schema 계약

### 현재 동작

ingress와 client server는 시작 시 `verify_ready`를 실행한다.

- `session_user=current_user`인지와 현재/전환 가능한 role의 특권, `public` schema CREATE 권한 및
  relation-owner membership을 검사한다.
- 필수 relation, column type/nullability/identity mode, constraint 이름/type/validated 상태와 운영 index의
  존재/valid/ready 상태를 검사한다.
- 모든 tenant table의 RLS/FORCE RLS를 검사한다.
- 정확한 policy 이름, permissive/command/PUBLIC 대상, `USING`/`WITH CHECK` 식을 검사한다.
- `_sqlx_migrations`의 version, success, checksum을 binary에 embedded된 migration과 비교한다.

client server의 `/health/ready`도 runtime schema를 다시 검사한다. ingress는 시작 시 검사는 하지만 현재
별도 HTTP readiness endpoint를 제공하지 않으므로 process/TCP/gRPC health와 startup 실패를 배포 probe로
조합해야 한다.

중요한 현재 제약은 **DB의 applied migration 집합이 binary의 embedded 집합과 정확히 같아야 한다**는
점이다. DB에 새 additive migration을 먼저 적용하면 이전 binary가 unexpected version으로 readiness에
실패하고, 새 binary를 먼저 띄우면 missing version으로 실패한다. 따라서 현재 v1 이후 schema 변경은
그대로는 rolling/zero-downtime migration이 아니며 아래의 현재 runbook처럼 runtime을 정지해야 한다.

### 계획: 호환성 window

무중단 migration을 시작하기 전에 먼저 readiness 계약을 다음 형태로 변경하는 migration-framework
release를 v1 schema에서 배포한다.

- binary는 자기가 알고 있는 migration의 checksum 변조/실패를 계속 거부한다.
- binary는 명시된 `[min_schema, max_compatible_schema]` 범위만 허용한다.
- additive migration을 선적용할 이전 binary도 그 migration version을 compatible future schema로
  인식하도록 한 release 앞서 배포한다.
- DB에 현재 schema version과 최소 호환 runtime contract를 나타내는 metadata를 두고 migrator가
  원자적으로 갱신한다.
- 구조 검증은 release가 실제로 사용하는 old/new column을 compatibility phase별로 검사한다.

이 항목은 **계획**이며 현재 코드에는 metadata table이나 range 기반 readiness가 없다. 이 기반을 구현하고
검증하기 전에는 expand/contract 절차를 무중단이라고 간주하지 않는다.

## migration artifact 규칙

현재 SQLx migrator는 `sqlx::migrate!("./migrations")`로
`skid-monitor-server/migrations/`의 SQL 파일을 compile time에 모두 발견하고 binary에 포함한다.
따라서 규칙에 맞는 새 SQL 파일을 추가하면 별도 Rust registry 수정 없이 migration 집합과 readiness
checksum 검증에 포함된다. `build.rs`는 migration directory 변경 시 재빌드를 강제하고 `.gitattributes`는
SQL을 LF로 고정해 OS별 checksum 차이를 막는다. build artifact가 바뀌지 않으면 runtime의 migration
집합도 바뀌지 않는다.

향후 artifact는 다음 규칙을 따른다.

1. `NNNN_lower_snake_case.sql` 형식과 단조 증가하는 정수 version을 사용한다. 예:
   `0002_add_event_retention_metadata.sql`.
2. 이미 어떤 환경에 적용된 파일은 절대 수정, 재정렬, squash하지 않는다. checksum mismatch는 장애로
   취급하고 수정 사항은 새 version으로 낸다.
3. migration은 forward-only이다. 일반적인 down migration 파일은 만들지 않는다. rollback은 호환되는
   application backout 또는 새 corrective forward migration이다.
4. 하나의 migration은 하나의 운영 목적을 가진다. 대용량 backfill과 schema DDL을 같은 transaction에
   넣지 않는다.
5. runtime query가 요구하는 grant, RLS enable/force, policy, constraint와 index도 versioned SQL에 포함한다.
6. PR에는 schema 전/후, lock 수준/예상 시간, table rewrite 가능성, disk/WAL 증가량, 호환 binary 범위,
   backfill과 backout 절차를 기록한다.
7. destructive SQL은 해당 column/table을 사용하지 않는 release가 모든 replica와 rollback 후보에서
   제거된 뒤 별도 contract migration으로 실행한다.

### transaction migration

일반 DDL은 SQLx의 transaction migration으로 실행한다. 실패 시 전체 transaction이 rollback되는 작은
metadata 변경, 새 table, nullable column, constraint `NOT VALID` 추가 등이 대상이다. 다음 항목을
사전에 검토한다.

- `ALTER TABLE`이 취득하는 lock과 production query 차단 시간.
- default expression이나 type 변경이 table rewrite를 발생시키는지.
- statement/lock timeout을 migration session에 설정했을 때 SQLx metadata 기록까지 일관되게 실패하는지.
- DDL transaction 동안 발생할 WAL, replica lag와 disk headroom.

### no-transaction migration

`CREATE INDEX CONCURRENTLY`, `DROP INDEX CONCURRENTLY`처럼 PostgreSQL이 transaction block 안에서
허용하지 않는 작업은 **그 작업만 포함한 별도 version**으로 두고 파일 첫 줄을 정확히
`-- no-transaction`으로 시작한다. 이 표식 앞에는 BOM, 빈 줄이나 다른 comment를 두지 않는다. SQLx 0.9가
파일 시작 문자열을 읽어 transaction 밖에서 실행한다. 다음 원칙을 지킨다.

- 동시에 일반 DDL, data update, grant를 섞지 않는다.
- 실패하면 partial/`INVALID` index가 남을 수 있으므로 `pg_index.indisvalid`와 `indisready`를 확인한다.
- 자동으로 `_sqlx_migrations` row를 수정하거나 success를 강제로 바꾸지 않는다.
- 승인된 recovery SQL로 partial object를 정리한 뒤 동일 artifact 재실행 가능 여부를 판단하거나 새
  corrective migration을 낸다.
- concurrent index가 준비된 뒤 별도 transaction migration에서 constraint에 attach하거나 query를
  전환한다.

## 계획: production expand/backfill/switch/contract

앞의 compatibility window가 구현된 뒤 모든 online schema 변경은 다음 네 단계를 독립 배포 단위로
수행한다. 현재 exact-version readiness에서는 이 절차를 무중단으로 사용할 수 없다.

### 1. Expand

- 기존 binary가 무시할 수 있는 nullable column, 새 table, 새 index를 먼저 추가한다.
- 큰 table의 새 column은 즉시 full rewrite가 일어나지 않도록 default와 `NOT NULL` 적용 방식을
  PostgreSQL version별로 확인한다.
- 새 FK/CHECK는 필요하면 `NOT VALID`로 추가해 짧은 lock으로 끝낸다.
- 큰 index는 별도 no-transaction `CREATE INDEX CONCURRENTLY` migration으로 만든다.
- old/new query 모두 필요한 grant와 RLS policy를 적용한다.
- schema migration 완료 뒤에도 기존 binary가 정상 read/write하는지 확인한다.

### 2. Backfill

- backfill은 migration transaction이 아니라 versioned Rust maintenance Job으로 수행한다.
- cursor/PK 범위의 작은 batch로 갱신하고 checkpoint를 남겨 재시작 가능하게 한다.
- 짧은 transaction, rate limit, lock timeout을 사용하고 replication lag, WAL, autovacuum, disk를 기준으로
  속도를 낮춘다.
- 여러 worker를 허용하면 `FOR UPDATE SKIP LOCKED` 또는 겹치지 않는 key range를 사용한다.
- tenant RLS를 유지할 수 있으면 tenant별 context로 실행한다. privileged role이면 대상 tenant와 row 수를
  별도 audit에 남긴다.
- null/old-format 잔여 row가 0이고 sample checksum/aggregate가 맞을 때만 완료로 판정한다.

### 3. Switch

- new binary를 canary replica에 먼저 배포한다.
- 전환 기간에는 새 write가 old/new representation을 모두 유지하거나, new reader가 old row에 fallback한다.
- canary에서 ingest idempotency, projection cursor, replay, tenant RLS, query latency/error를 확인한다.
- 전체 replica를 전환한 뒤 최소 한 rollback 관찰 window 동안 old schema path를 유지한다.
- feature flag가 필요하면 DB migration과 분리해 단계적으로 enable한다.

### 4. Contract

- 이전 binary가 완전히 제거되고 rollback 대상도 새 schema를 이해함을 확인한다.
- dual write/backfill lag가 0이고 보존된 backup/PITR 시점이 있음을 확인한다.
- `NOT VALID` constraint는 먼저 `VALIDATE CONSTRAINT`하고, `NOT NULL` 등 최종 제약은 lock 시간을 검증한
  별도 migration으로 적용한다.
- 폐기 column/index/table은 마지막 별도 migration에서 제거한다. 대형 object drop의 lock과 disk 회수
  방식도 사전 시험한다.
- contract 뒤에는 application rollback 대신 corrective forward migration을 기본 backout으로 사용한다.

## 배포 전 preflight와 canary

Migration Job 실행 전에 다음을 모두 만족해야 한다.

- 배포할 migrator와 runtime image digest, migration version/checksum이 승인된 artifact와 같다.
- staging 또는 production snapshot clone에서 migration, postflight, 새 binary, 이전 binary 호환 시험을
  통과했다.
- managed PostgreSQL engine version이 프로젝트 support matrix 안에 있다. 현재 저장소에는 이 matrix가
  정의되어 있지 않으므로 production 도입 전에 명시해야 한다.
- primary/replica health, replication lag, available storage, WAL 증가 여유, connection headroom이 정상이다.
- 장기 transaction, blocking lock, 진행 중인 maintenance/backup/DDL이 없다.
- 최신 backup 성공과 PITR 복원 가능 시점을 확인했다.
- migration role은 예상 role이고 runtime Secret과 다르며, runtime role은 직접 login이고 특권 role
  membership, DDL/superuser/BYPASSRLS/CREATEDB/CREATEROLE/REPLICATION이 없다.
- 예상 row 수, index 크기, backfill batch와 lock/statement timeout을 기록했다.

Canary는 별도 tenant와 실제와 같은 signal 크기를 사용한다. 최소 검증 항목은 다음과 같다.

- agent 등록 후 같은 `(tenant, agent, sequence, payload)`를 두 번 보내도 event/projection이 한 번만
  증가한다.
- 같은 sequence의 다른 payload가 거부된다.
- 두 tenant의 direct/replay query가 서로의 row를 보지 못한다.
- cursor 순서, byte-bounded replay와 WebSocket fallback poll이 동작한다.
- disabled tenant/agent가 거부되고 audit/ticket 일회 소비가 유지된다.
- query p95/p99, lock wait, DB CPU/IO, replica lag와 error rate가 배포 전 기준에서 허용 범위다.

## rollback, backout과 실패 복구

DB snapshot 복원은 일상적인 application rollback이 아니다. 다른 tenant의 정상 write까지 되돌리므로
데이터 손상/재난 복구 상황에만 승인된 PITR runbook으로 사용한다.

| 실패 시점 | 기본 대응 |
|---|---|
| migration 시작 전 preflight 실패 | Job을 실행하지 않고 원인을 제거한다. runtime은 그대로 유지한다. |
| transaction migration 실패 | rollback 여부와 `_sqlx_migrations` 상태를 확인한다. SQL 파일을 수정하지 말고 원인을 고친 새 artifact/재실행 계획을 승인한다. |
| no-transaction migration 실패 | partial/invalid object와 SQLx history를 조사하고 승인된 cleanup 후 재실행 또는 corrective migration을 사용한다. |
| expand 뒤 새 binary 장애 | old binary로 application backout한다. additive schema는 유지한다. |
| backfill 중 실패 | Job을 중지하고 checkpoint부터 재개한다. 이미 변환된 row가 재처리에 안전해야 한다. |
| switch 뒤 오류율/latency 증가 | feature flag를 되돌리고 compatible old binary로 축소한다. contract는 실행하지 않는다. |
| contract 뒤 회귀 | old binary rollback을 시도하지 않는다. corrective forward migration/new binary를 낸다. 데이터 손상이면 PITR incident 절차로 전환한다. |
| disk/WAL/replica lag 임계 초과 | backfill/concurrent index를 중단 또는 throttle하고 storage/replica가 회복될 때까지 진행하지 않는다. |
| LISTEN 연결/HA failover | listener가 재연결되는 동안 2초 cursor poll이 데이터 전달을 보완하는지 확인한다. |

사고 중에는 `_sqlx_migrations`의 version/success/checksum을 임의 UPDATE/DELETE하지 않는다. 꼭 metadata
수정이 필요하다면 incident commander와 DB owner 승인, 원본 snapshot, 실행 SQL과 사후 검증을 남긴다.

## 현재 v1 변경 runbook

호환성 window가 구현되기 전 v1에서 새 migration을 추가하는 안전한 절차는 **정지 배포**이다.

1. 변경 freeze를 선언하고 ingress를 drain/scale-to-zero하여 신규 write를 멈춘다.
2. client stream을 종료하고 client server를 scale-to-zero한다.
3. active transaction/connection과 최신 backup/PITR 시점을 확인한다.
4. 승인된 image의 `skid-monitor-migrate` Job을 migration credential로 한 번 실행한다.
5. Job exit code, `_sqlx_migrations` version/success/checksum과 schema/grant/RLS postflight를 확인한다.
6. 정확히 같은 embedded migration 집합을 가진 ingress/client image를 배포한다.
7. startup/readiness, Keycloak auth, tenant RLS와 canary ingest/replay를 검증한다.
8. client server를 먼저 소수 replica로 올려 read path를 확인하고, ingress를 canary부터 재개한다.
9. 관찰 window 동안 DB/error 지표를 확인한 뒤 전체 replica를 복구한다.

정지 시간을 허용할 수 없다면 schema를 변경하지 말고 먼저 앞의 호환성 readiness release를 구현한다.

## 향후 online migration runbook

호환성 window 구현 뒤에는 다음 배포 순서를 사용한다.

1. old schema와 다음 additive schema를 모두 허용하는 bridge binary를 전 replica에 배포한다.
2. expand transaction migration과 필요한 concurrent-index migration을 순서대로 실행한다.
3. old binary 호환성과 readiness를 postflight로 확인한다.
4. resumable backfill Job을 실행하고 완료 조건을 검증한다.
5. new reader/writer를 canary 후 전체 배포하고 dual path/feature flag 지표를 관찰한다.
6. rollback window가 끝날 때까지 additive schema와 old data를 유지한다.
7. contract migration 준비 review와 새 backup을 거친 뒤 obsolete object를 제거한다.
8. schema compatibility floor를 올리고 문서/support matrix를 갱신한다.

각 단계는 독립적으로 중단할 수 있어야 하며 한 배포 Job이 expand부터 contract까지 연속 실행해서는 안
된다.

## grant와 postflight 검증

새 relation/sequence를 만드는 migration은 같은 phase에서 owner와 grant를 확인한다. production
postflight는 적어도 다음을 검사한다.

- 모든 application relation의 owner가 의도한 owner role이다.
- runtime role에 schema `CREATE`, relation-owner/특권 role membership,
  superuser/BYPASSRLS/CREATEDB/CREATEROLE/REPLICATION이 없고 `session_user=current_user`이다.
- ingress/client role의 table/sequence grant가 앞의 matrix와 일치한다.
- tenant relation은 RLS와 FORCE RLS가 모두 켜져 있고 정확한 isolation policy가 있다.
- `_sqlx_migrations`의 모든 known version이 성공이고 embedded checksum과 같다.
- invalid index, unvalidated 상태로 남으면 안 되는 constraint, null backfill 잔여가 없다.
- 서비스 `verify_ready`가 실제 runtime credential로 통과한다.

저장소의 PostgreSQL integration test는 다음처럼 명시적으로 provision한 DB에서 실행한다.

```sh
export SKID_MONITOR_TEST_DATABASE_URL='postgresql://.../skid_monitor_test?sslmode=verify-full'
cargo test -p skid-monitor-server --test postgres_store \
  postgres_store_is_idempotent_projected_and_tenant_isolated -- --ignored --exact
```

현재 이 test는 기본 test run에서 ignored이며 자동으로 production PostgreSQL/Keycloak E2E를 수행하지
않는다. CI에서는 disposable DB에 v1부터 최신까지 migration한 경우와, 지원하는 이전 schema snapshot에서
순차 upgrade한 경우를 모두 추가해야 한다.

## HA, backup, PITR와 복구 훈련

다음은 **운영 구성**이다.

- production primary는 가능하면 managed multi-AZ synchronous standby와 자동 failover를 사용한다.
- application write/replay는 현재 primary endpoint를 사용한다. read replica routing은 cursor 일관성과
  replica lag 처리가 구현되기 전에는 사용하지 않는다.
- automated backup과 continuous WAL/PITR를 활성화하고 backup을 DB 계정/cluster와 다른 장애 경계에
  보관한다.
- 조직은 RPO, RTO, backup retention과 법적 삭제 요구를 숫자로 승인해야 한다. 출발점 예시는
  `RPO <= 5분`, `RTO <= 60분`, `PITR >= 14일`이지만 실제 SLO/규제에 따라 확정한다.
- 분기마다 격리된 새 cluster로 PITR restore하고 schema checksum, tenant row 수/sample hash, RLS,
  canary ingest/replay를 검증한다. backup 성공 알림만으로 복구 가능성을 판정하지 않는다.
- failover 훈련에서는 pool/listener 재연결, OTLP retry idempotency, cursor replay와 실제 RTO를 측정한다.

보조 `pg_dump`가 필요하면 managed physical/PITR backup을 대체하지 않는다. RLS로 tenant가 누락되지 않도록
backup role과 dump 결과를 검증하고, role/grant/policy 같은 cluster-level 복구 자료도 별도로 versioning한다.

## retention과 maintenance

현재 `signal_events`와 `audit_events`에는 TTL이나 partition이 없고, `stream_tickets`도 tenant가 새 ticket을
만들 때만 부분 정리된다. 따라서 production 전에 다음을 결정해야 한다.

- signal 원본, audit, ticket 각각의 보존 기간과 tenant 삭제 정책.
- tenant별 용량/quota와 과금/법적 hold.
- 시간 partition 사용 여부, partition create/drop 선행 일정과 late-arriving event 처리.
- partition 전환 전까지 사용할 작은 batch delete, vacuum와 bloat 관리.
- 누적 `signal_projection`을 event retention 뒤에도 유지할지, 보존 창 projection을 별도로 만들지.

Retention Job은 **계획**이며 runtime service 안의 background task로 숨기지 않는다. maintenance credential,
명시적 schedule/concurrency, dry-run, 대상 기간/row 수 audit, statement/lock timeout과 중단 가능한 batch를
가진 별도 Rust Job으로 구현한다. 대량 `DELETE`보다 검증된 partition drop을 선호하되 현재 table을 즉시
partition으로 바꾸는 것은 별도 expand/backfill/switch 프로젝트로 다룬다.

## observability와 경보

애플리케이션과 managed PostgreSQL에서 다음을 tenant payload나 token을 로그에 남기지 않고 수집한다.

- pool 사용/idle/waiter, acquire timeout, query error와 transaction duration.
- ingest commit latency/rate, idempotent retry, sequence conflict, projection update latency.
- replay query row/byte 수, WebSocket backlog/send timeout, fallback poll, listener reconnect/notification parse 오류.
- DB connection utilization, CPU, memory, IOPS/latency, storage 사용/증가율, WAL 생성률.
- lock wait/deadlock, 장기 transaction, autovacuum 지연, table/index bloat.
- replica lag/failover, backup/PITR 성공과 마지막 restore drill 시각.
- migration version/checksum/readiness 실패, migration duration, backfill 진행률/ETA/error.

필수 경보는 storage/WAL 고갈 예측, connection 포화, 지속 lock wait, replica lag, backup 실패, readiness
불일치와 migration 실패이다. migration과 maintenance 배포에는 변경 전/후 dashboard snapshot과 담당자,
runbook link를 남긴다.

## 구현 상태 요약

| 영역 | 현재 | 다음 production 작업 |
|---|---|---|
| Event/projection/ticket/audit schema | v1 구현 | workload 기반 index/partition 검증 |
| Tenant isolation | FORCE RLS + exact readiness 구현 | role/grant provisioning SQL과 CI 검사 |
| Migration runner | 별도 Rust binary, 디렉터리 자동 포함, checksum 검증 구현 | compatibility metadata와 range 기반 readiness 구현 |
| Rolling migration | 미지원 | bridge readiness 후 expand/contract 활성화 |
| LISTEN/NOTIFY | cursor wake-up + 2초 poll 구현 | notification/lag 운영 지표와 규모 시험 |
| HA/backup/PITR | application 외부 책임 | managed 정책, 수치화한 RPO/RTO, restore drill |
| Retention/backfill | 자동화 없음 | 별도 Rust maintenance Job과 partition 계획 |
| PostgreSQL E2E | ignored integration test 존재 | disposable CI DB와 실제 staging Keycloak/PG 검증 |
