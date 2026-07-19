# Getting Started

| 항목 | 값 |
| --- | --- |
| Status | Experimental |
| Applies to | v0.1.x |
| Last verified | 2026-07-19 |
| Platforms verified | macOS arm64 runtime; Linux parser tests only |

이 문서는 별도 database나 identity provider 없이 native frontend와 agent를 한 machine에서 실행하는
trusted-local Solo 경로를 다룬다.

## 완료 조건

아래 절차가 성공하면 frontend의 `Agents` 표에 local agent가 나타난다. 해당 행의 `Open`을 누르면
`Latest Metrics`에 host metric이 표시되고, overview의 `last seen`은 약 15초마다 갱신된다.

## Prerequisites

- Git
- Rust 1.94 이상과 Cargo
- native window를 열 수 있는 graphical desktop session
- 첫 build에서 crates를 내려받을 network access

workspace는 Rust edition 2024를 사용한다. 현재 dependency graph에서 SQLx 0.9가 Rust 1.94를
요구하므로 더 낮은 compiler는 지원 대상으로 간주하지 않는다. Windows host sampler는 아직
구현되지 않았으므로 이 Quick Start의 검증 platform이 아니다.

버전을 확인한다.

```sh
git --version
rustc --version
cargo --version
```

## 1. Clone

```sh
git clone https://github.com/LuticaCANARD/skid-monitor.git
cd skid-monitor
```

## 2. Frontend 실행

첫 terminal에서 native control room을 실행한다. Solo receiver는 numeric loopback address만 허용한다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-fe
```

창이 열리면 `Ingress listeners`에 `127.0.0.1:9000`이 표시되는지 확인한다. frontend가 먼저 port를
listen해야 agent의 첫 signal도 놓치지 않는다.

## 3. Agent 실행

두 번째 terminal에서 같은 checkout으로 이동해 agent를 실행한다.

```sh
cd skid-monitor
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

더 자세한 startup log가 필요하면 `RUST_LOG=info`를 추가한다.

```sh
RUST_LOG=info \
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 \
cargo run -p skid-monitor-agent
```

## 4. Verify

frontend에서 다음을 순서대로 확인한다.

1. `Ingress listeners`에 `127.0.0.1:9000`이 있다.
2. `Agents`에 endpoint가 `127.0.0.1:9000`인 행이 생긴다.
3. macOS는 `macos`, Linux는 `system` source의 host metric 행을 만든다.
4. host metric 행의 `Open`을 누르면 `Latest Metrics`에 `system.*` 또는 `macos.*` metric이 보인다.
5. 최대 15초를 기다렸을 때 counters와 `last seen`이 다시 갱신된다.

2026-07-19 macOS arm64 smoke test에서는 agent 첫 cycle이 22개 metric을 수집했고, frontend SQLite
state에 `macos` metric node와 log node가 등록되는 것을 확인했다. 이 수치는 hardware와 OS에 따라
달라지므로 정확한 개수보다 갱신 여부를 성공 기준으로 삼는다.

## 5. Stop

frontend 창을 닫고 agent terminal에서 `Ctrl-C`를 누른다. agent는 shutdown signal을 받은 뒤
telemetry provider를 flush하고 종료한다.

## Optional verification

PostgreSQL이 필요 없는 기본 test suite는 다음과 같다.

```sh
cargo test --workspace
```

PostgreSQL store의 tenant isolation, idempotency, cursor와 projection 통합 test는 준비된 test
database에만 실행한다.

```sh
TEST_DATABASE_URL='postgresql://...' \
  cargo test -p skid-monitor-server --test postgres_store -- --ignored
```

운영 database나 owner/superuser credential을 test에 사용하지 않는다.

## Failure modes

### `Address already in use`

다른 process가 port 9000을 사용 중이다. 두 command 모두 같은 빈 loopback port로 바꾼다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9200 cargo run -p skid-monitor-fe
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9200 cargo run -p skid-monitor-agent
```

### 창은 열리지만 agent가 나타나지 않음

- frontend를 agent보다 먼저 실행했는지 확인한다.
- 두 process의 `SKID_MONITOR_CLIENT_ADDR`가 정확히 같은지 확인한다.
- agent를 `RUST_LOG=info`로 다시 실행하고 `client로 TCP 전송 실패`가 있는지 본다.
- sandbox/container가 loopback bind/connect를 금지하면 host에서 실행한다.

Solo path에는 durable payload spool이 없다. frontend가 꺼져 있을 때 실패한 signal은 나중에 replay되지
않고, 다음 15초 cycle부터 새 signal 전송을 다시 시도한다.

### Linux graphics initialization failure

기본 frontend는 low-spec software GL 경로를 우선한다. driver/Wayland 문제와 GPU·Wayland opt-in은
[frontend README](../client/skid-monitor-fe/README.md)의 Linux 항목을 확인한다.

### `cargo run -p skid-monitor-client`가 실패함

`skid-monitor-client`는 frontend가 공유하는 library crate이며 bin target이 아니다. 사람이 실행하는
native client는 `cargo run -p skid-monitor-fe`다.

## Next paths

- agent pipeline과 DB log 설정: [agent example config](../skid-monitor-agent/examples/agent-config.json)
- PostgreSQL/OIDC Cloud mode: [Cloud and Solo Deployment](cloud-solo-deployment.md)
- multi-listener와 browser frontend: [frontend README](../client/skid-monitor-fe/README.md)
- 현재 제약과 검증 근거: [Feature Status](feature-status.md)
