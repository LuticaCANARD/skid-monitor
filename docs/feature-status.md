# Feature Status

| 항목 | 값 |
| --- | --- |
| Status | Living implementation inventory |
| Applies to | v0.1.x |
| Last verified | 2026-07-19 |
| Platforms verified | macOS arm64 runtime; workspace tests on macOS arm64 |

이 문서가 제품 기능 상태의 정준 출처다. RFC의 목표나 crate 이름만으로 구현 완료를 추론하지 않고,
code와 test가 제공하는 현재 경계를 기록한다.

## 상태 정의

| 상태 | 의미 |
| --- | --- |
| Stable | 기본 경로가 구현되고 자동 test와 지원 platform 실행 검증이 있으며 호환성 약속이 있음 |
| Experimental | 실제로 작동하지만 API, 설정, 동작 또는 운영 계약이 바뀔 수 있음 |
| Prototype | 제한된 scenario 또는 placeholder만 구현되어 제품 경로로 일반화할 수 없음 |
| Planned | 사용할 수 있는 runtime 구현 없이 설계, RFC 또는 type reservation만 존재함 |

현재 Stable로 선언한 기능은 없다. 상태를 올릴 때는 관련 code, automated test, 실행 검증 platform을
함께 갱신해야 한다.

## 현재 기능

| 기능 | 상태 | 지금 구현된 범위 | 검증 근거 | 남은 경계 |
| --- | --- | --- | --- | --- |
| Native Solo dashboard | Experimental | numeric loopback TCP, metrics/logs/traces UI, SQLite node/alert state | [native ingress](../client/skid-monitor-fe/src/platform/ingress/native.rs), [receiver tests](../client/skid-monitor-client/src/test/receiver.rs), [macOS smoke screenshot](assets/skid-monitor-control-room.jpg) | raw signal replay, installer, automated UI smoke |
| Multi-agent frontend | Experimental | 여러 listener의 node를 한 control room에서 분리 표시하고 runtime bind/unbind | [receiver loop](../client/skid-monitor-client/src/receiver_loop.rs), [lifecycle tests](../client/skid-monitor-fe/src/state/lifecycle.rs) | authenticated remote native ingress, cross-listener ordering contract |
| OTLP metrics/logs/traces ingress | Experimental | agent의 tonic gRPC receiver가 세 signal을 pipeline에 투입 | [receiver](../skid-monitor-agent/src/otlp_receiver.rs), [pipeline](../skid-monitor-agent/src/pipeline.rs) | live SDK end-to-end CI, health endpoint |
| Linux host metrics | Experimental | `/proc` 기반 CPU, load, memory, filesystem, disk, network, process sampler | [sampler and parser tests](../skid-monitor-agent/src/system_metrics.rs) | 이번 검증에서 Linux runtime smoke 미실행, package/service test |
| macOS host metrics | Experimental | `uptime`, `vm_stat`, `df`, `pmset` 기반 sampler와 `Source::MacOS` | [sampler and tests](../skid-monitor-agent/src/system_metrics.rs), 2026-07-19 arm64 smoke | IOKit/sysctl 확대, signed package/service test |
| Windows host metrics | Planned | runtime sampler와 `Source::Windows`가 없음 | [source enum](../skid-protocol/src/metrics.rs), [native deployment target](agent-continuous-deployment.md) | PDH/WMI/CIM/ETW/Event Log adapter와 Windows runner |
| Database log receiver | Experimental | file tail, start position, truncate/rotation, partial/oversized line 처리, OTLP Logs metadata | [implementation and tests](../skid-monitor-agent/src/database_logs.rs), [config example](../skid-monitor-agent/examples/agent-config.json) | restart-safe checkpoint, multiline/timestamp parser, redaction processor |
| PostgreSQL/OIDC Cloud mode | Experimental | split ingress/client API, OAuth/OIDC roles, tenant RLS, idempotent append, cursor replay | [server](../skid-monitor-server/src), [migration](../skid-monitor-server/migrations/0001_cloud_signal_store.sql), [conditional integration test](../skid-monitor-server/tests/postgres_store.rs) | live DB test는 기본 suite에서 ignored, retention/restore/HA deployment 검증 |
| Out-of-process .NET extensions | Experimental | Rust receiver가 newline-delimited JSON을 .NET sidecar stdin으로 전달 | [Rust host boundary](../client/skid-monitor-client/src/extension.rs), [.NET guide](../client/skid-monitor-client/bindings/dotnet/README.md) | 이번 검증에서 dotnet build/runtime smoke 미실행, backpressure policy |
| Edge collection | Prototype | compact no-std wire decode와 deterministic mock temperature/voltage/RSSI sender | [wire tests](../skid-edge-wire/src/lib.rs), [mock sender](../skid-edge-agent/src/main.rs), [device adapter test](../skid-monitor-agent/src/device_socket.rs) | 실제 GPIO/I2C/serial/MCU sensor와 enrollment/auth |
| Deterministic alerts / configurable character | Prototype | 고정 threshold alert의 `idle`/`warning`/`critical` 상태를 `Still`/`Pulse`/`Bounce`/`Shake` motion과 상태별 message에 매핑한다. Character panel은 low/high-spec 공통이며 native PNG/JPEG를 읽는다. native high-spec은 VRM 0.x/1.0 embedded mesh, rest skin, base-color texture를 depth-enabled WGPU viewport에 정적으로 표시한다. profile은 native SQLite 또는 tenant/legacy-scoped browser `localStorage`에 저장한다. | [alert tests](../client/skid-monitor-fe/src/alert.rs), [reaction profile](../client/skid-monitor-fe/src/model/avatar.rs), [presenter](../client/skid-monitor-fe/src/components/avatar.rs), [VRM loader](../client/skid-monitor-fe/src/components/avatar/vrm/loader.rs) | heartbeat/offline detection, 사용자 정의 alert threshold/rule, browser local-file import, expression/MToon 전체/SpringBone/VRMA와 skeletal animation clip player |
| Authorized file transfer | Planned | 별도 node가 root availability/file count/bytes metadata만 전송 | [current node](../skid-file-node/src/main.rs), [design rationale](../skid-file-node/docs/rfcs/0001-crate-role.md) | offer, auth, path/symlink policy, chunk/hash/resume data plane |
| Compute routing | Prototype | logical CPU, `gpu.detected=0`, placeholder score를 보고 | [current advisor and test](../skid-compute-advisor/src/main.rs), [role RFC](../skid-compute-advisor/docs/rfcs/0001-crate-role.md) | GPU/memory/thermal detection, scoring model; remote execution은 non-goal |
| Quantum backend adapter | Planned | `Source::Quantum` type reservation만 있고 provider API adapter는 없음 | [source enum](../skid-protocol/src/metrics.rs), [umbrella RFC](rfcs/0001-initial-skid-monitor-integration.md) | provider job API adapter, identity/config/test |

## Verification snapshot

2026-07-19에 native FE + agent smoke를 확인했고, 2026-07-20에 자동 검증을 다시 실행했다.

- `cargo test --workspace`: 170 passed, 0 failed, PostgreSQL integration test 1개 ignored
- `cargo test -p skid-monitor-fe --lib --no-default-features --features high-spec`: 76 passed, 0 failed
- `cargo check -p skid-monitor-fe --target wasm32-unknown-unknown --no-default-features --features web`: passed
- 공식 Seed-san VRM 1.0 sample: loader decode 후 Apple M1 Metal의 512x512 offscreen WGPU
  `Rgba8Unorm` + `Depth24Plus` draw에서 validation error 없이 visible pixel 확인
- macOS arm64 native FE + agent smoke: loopback signal 수신, `macos` metric node와 log node SQLite 등록
- agent first cycle: host/self-observation metric, trace, log batch 생성
- PostgreSQL, Linux runtime, Windows, .NET extension, actual edge hardware는 이 검증에서 실행하지 않음

ignored test를 실행하지 않은 상태에서 PostgreSQL 통합이 검증됐다고 표현하지 않는다. 자동화된
README Quick Start smoke가 추가되기 전에는 native dashboard도 Stable로 올리지 않는다.
