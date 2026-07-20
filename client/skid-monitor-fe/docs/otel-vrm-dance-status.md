# OpenTelemetry 신호 기반 VRM/VTuber 댄스 연동 현황

## 결론

프론트엔드는 고정 alert severity에 반응하는 사용자 설정형 Character presenter를 제공한다. 사용자는
`idle`, `warning`, `critical` 상태마다 `Still`, `Pulse`, `Bounce`, `Shake` motion과 말풍선 message를
설정할 수 있다. native `high-spec` 빌드는 사용자 `.vrm` 파일의 VRM 1.0 또는 legacy 0.x 구조를 검증하고,
embedded mesh, node hierarchy, rest-pose skin, base-color texture를 WGPU 3D viewport에 표시한다.

Character panel과 built-in/PNG/JPEG 경로는 `low-spec`과 `high-spec` 모두에서 동작한다. `low-spec`에서
VRM path를 선택하거나 VRM 로딩이 실패하면 built-in 2D character로 fallback한다. 현재 VRM은 정적
rest-pose renderer이며 expression morph, MToon 고유 shading, SpringBone, constraint/lookAt, VRMA/dance
clip 실행은 구현하지 않았다.

즉, 지금 바로 있는 것을 기준으로 보면 다음처럼 판단할 수 있다.

| 항목 | 현재 상태 | 근거 |
| --- | --- | --- |
| OpenTelemetry/OTLP 수신 | 구현됨 | `skid-monitor-agent`가 OTLP gRPC metrics/traces/logs receiver를 제공한다. |
| agent에서 client/frontend로 신호 전달 | 구현됨 | `Signal::{Metrics, Traces, Logs}`를 length-prefixed JSON TCP frame으로 보낸다. |
| egui frontend의 신호 표시 | 구현됨 | `skid-monitor-fe`가 metrics/traces/logs를 받아 dashboard, counter, event log로 표시한다. |
| high-spec VRM renderer | Prototype | `high-spec` feature가 WGPU depth viewport에서 native VRM 0.x/1.0 mesh, rest skin, base-color texture를 표시한다. |
| 2D Character presenter | 구현됨 | low/high-spec 공통 Character panel이 built-in 또는 native PNG/JPEG 모델을 표시한다. |
| 사용자 reaction profile | 구현됨 | `idle`/`warning`/`critical`별 Still/Pulse/Bounce/Shake와 custom message를 설정한다. |
| Character profile 영속화 | 구현됨 | native는 SQLite, browser는 인증된 cloud tenant scope 또는 legacy local scope의 `localStorage` key에 profile을 저장한다. |
| 알람 -> VRM 상태 설계 | 문서화됨 | RFC 0002/0004에 `AlertSnapshot`, short message, VRM presenter 전달이 정의되어 있다. |
| VRM animation/runtime 전체 | 부분 구현 | 정적 3D 모델 표시까지 구현했다. expression, SpringBone, MToon 고유 효과, VRMA/animation clip은 없다. |
| 알람 -> character 상태 변환 | 구현됨 | 선택 Agent의 최고 fixed severity를 idle/warning/critical action에 매핑한다. custom threshold rule은 없다. |
| Unity/VRM bridge 경로 | 부분 구현 | Rust client가 .NET extension host로 raw `Signal` JSON을 전달할 수 있다. |

## 현재 신호 흐름

현재 동작하는 기본 흐름은 아래와 같다.

```text
OpenTelemetry app / system sampler / device node
        |
        v
skid-monitor-agent
  - OTLP gRPC receiver
  - self observation collector
  - device socket receiver
        |
        v
Signal::{Metrics, Traces, Logs}
        |
        v
skid_client TCP exporter
        |
        v
skid-monitor-fe
  - dashboard counters
  - metrics table
  - trend graph
  - event log
  - fixed alert evaluation
  - configurable Character reaction
  - native high-spec VRM 3D viewport
```

이 흐름은 "관측 신호를 UI까지 가져오는 경로"로는 이미 쓸 수 있다. OpenTelemetry application이
agent의 OTLP endpoint로 export하면, agent pipeline이 `skid` exporter를 통해 frontend/client가
듣는 주소로 `Signal`을 보낸다.

주의할 점은 frontend가 OTLP receiver 자체는 아니라는 것이다. frontend는 `SKID_MONITOR_CLIENT_ADDR`
로 들어오는 Skid `Signal` TCP frame을 받는다. OTLP는 agent가 받는다.

## 현재 Character 경로

현재 Character presenter는 alert core와 분리된 표현 계층이다. 내장 alert rule이 선택 Agent의 최고
severity를 계산하면 reaction profile이 다음 중 하나를 고른다.

| Alert 상태 | Reaction profile | 지원 motion |
| --- | --- | --- |
| active alert 없음 | `idle` | Still, Pulse, Bounce, Shake |
| warning | `warning` | Still, Pulse, Bounce, Shake |
| critical | `critical` | Still, Pulse, Bounce, Shake |

각 상태에는 사용자가 지정한 말풍선 message를 함께 표시할 수 있다. motion은 2D sprite 또는 VRM
viewport를 안전하게 이동하거나 크기 변화시키는 bounded UI effect다. skeletal animation, arbitrary
script, VRMA 또는 embedded/external animation clip 실행이 아니다.

- native frontend: `.png`, `.jpg`, `.jpeg`, `.vrm` filesystem path를 선택할 수 있고 profile은 SQLite
  write ACK 이후 적용한다. model decode는 크기를 제한한 단일 background loader에서 직렬 처리한다.
- native `high-spec`: `.vrm`을 GLB로 파싱한다. VRM 1.0은 필수 `meta.name`/`authors`/`licenseUrl`과
  humanoid bone을, legacy 0.0은 별도 필수 bone 집합과 meta object의 알려진 field type을 검증한다.
  external buffer/image URI와 unsupported required extension은 거부한다. node transform, embedded
  base-color texture, static rest skinning을 GPU에 올린다.
- native `low-spec`: VRM loader/renderer 의존성을 포함하지 않으며 `.vrm` 선택 시 built-in fallback과
  high-spec 안내를 표시한다.
- browser frontend: profile은 signal/alert와 같은 인증된 cloud tenant scope(또는 legacy local scope)의
  `localStorage` key에 저장한다. 인증 전 cloud pending 상태에서는 저장하지 않으며, browser가 native
  filesystem path를 읽을 수 없으므로 built-in character를 유지한다.
- 모델 path가 비어 있거나 model load가 실패하면 built-in character로 fallback한다.
- alert threshold와 severity는 현재 내장 rule을 그대로 사용하며 Character 설정에서 변경하지 않는다.
  선택 server의 metric alert와 해당 listener의 receiver error만 Character state에 반영한다. frontend
  extension-host error는 alert/event UI에는 남지만 특정 server의 상태로 매핑하지 않는다.

## Native high-spec VRM renderer

`skid-monitor-fe`는 egui native app이며 `high-spec` feature에서 WGPU renderer와 24-bit depth buffer를
선택한다.

```sh
cargo run -p skid-monitor-fe --no-default-features --features high-spec
cargo test -p skid-monitor-fe --lib --no-default-features --features high-spec
```

`Settings > Character reactions`에서 `.vrm` path를 입력하거나 native window에 `.vrm`을 drop한 뒤
profile을 Apply한다. loader는 파일을 최대 128 MiB로 제한하고, node/primitive/triangle/texture 및 decoded
texture allocation에도 상한을 둔다. CPU parsing은 기존 single background loader에서 실행하고, GPU
resource 생성은 egui WGPU callback의 prepare 단계에서만 수행한다. stale generation은 설치하지 않는다.

MToon material은 glTF base-color PBR/unlit 호환 정보로 fallback한다. 정적 rest-pose skinning까지만
적용하며 expression morph, blink/lookAt, SpringBone, node constraint, VRMA와 glTF animation은 실행하지
않는다. `.glb` 일반 모델은 VRM으로 가장하지 않도록 profile validation에서 허용하지 않는다.

## 외부 Unity/VRM 연결점

`skid-monitor-client`에는 out-of-process .NET extension host가 있다. Rust client/frontend 수신
경로에서 받은 `Signal`을 newline-delimited JSON으로 sidecar process의 stdin에 보낼 수 있다.

현재 envelope는 다음 형태다.

```json
{
  "schema": "skid.monitor.extension.v1",
  "type": "signal",
  "signal": {}
}
```

이 경로는 Unity companion client와 연결하기에 현실적이다.

```text
skid-monitor-fe / skid-monitor-client
        |
        v
.NET extension host
        |
        v
C# extension: Signal -> avatar state event
        |
        v
Unity companion client + UniVRM
        |
        v
VRM character animation / dance / expression
```

문서상 권장되는 Unity event contract는 다음처럼 raw OTLP 전체가 아니라 작은 avatar event를 보내는
방식이다.

```json
{
  "schema": "skid.monitor.avatar.v1",
  "state": "thermal_warning",
  "severity": 0.72,
  "source": "edge_device",
  "title": "Rack A temperature rising",
  "attributes": {
    "device_id": "edge-01",
    "sensor": "temperature"
  }
}
```

현재 구현된 것은 raw `Signal` 전달까지다. 위 `skid.monitor.avatar.v1` 이벤트 생성, WebSocket/TCP
relay, Unity 쪽 수신기, UniVRM Animator mapping은 아직 없다.

## OpenTelemetry 신호로 춤추게 하는 해석 계층

VRM이 춤추려면 OpenTelemetry payload를 바로 animation clip에 연결하기보다, 중간에 작은 상태
계층을 두는 편이 안전하다.

권장 구조는 다음과 같다.

```text
Signal
  -> signal summary
  -> health / alert / rhythm state
  -> avatar state event
  -> renderer-specific animation
```

현재 presenter와 향후 VRM/dance 경로를 구분하면 다음과 같다.

| OpenTelemetry/alert 조건 | Character state | 현재 범위 |
| --- | --- | --- |
| active alert 없음 | `idle` | 설정한 sprite/VRM viewport motion과 message 실행 |
| warning alert firing | `warning` | 설정한 sprite/VRM viewport motion과 message 실행 |
| critical alert firing | `critical` | 설정한 sprite/VRM viewport motion과 message 실행 |
| alert 해소 | `idle`로 복귀 | 별도 `relieved` one-shot action은 미구현 |
| metric 유입량/latency/rhythm | `active`/`degraded`/`dance_loop` | classifier와 dance runtime 모두 미구현 |

댄스는 알람 표현과 충돌하지 않게 우선순위를 둬야 한다.

1. `critical`: 춤보다 경고 pose/gesture가 우선이다.
2. `warning`: dance loop를 약하게 줄이거나 attention gesture를 섞는다.
3. `normal/active`: metric rhythm에 맞춘 dance loop를 재생할 수 있다.
4. `resolved`: 짧은 회복 motion 뒤 idle/dance로 돌아간다.

## 현재 코드 기준 구현 공백

현재 high-spec renderer는 VRM을 정적으로 표시한다. OpenTelemetry 신호가 실제 VRM/VTuber skeletal
animation이나 dance까지 가려면 아래 항목이 추가되어야 한다.

1. generic `Signal` 요약기
   - 현재 Character는 선택 server의 fixed metric alert와 해당 listener의 receiver alert만 사용한다.
   - traces/logs/rhythm/latency를 `active`, `degraded`, `dance_loop`로 분류하는 기능은 없다.

2. 사용자 정의 alert rule engine
   - CPU/memory/file-root threshold와 receiver/extension severity는 코드에 고정되어 있다. extension
     alert는 특정 server Character state로 사용하지 않는다.
   - Character profile은 그 결과의 motion/message만 바꾸며 threshold나 rule 조건을 바꾸지 않는다.

3. avatar event contract
   - `skid.monitor.avatar.v1` JSON schema와 renderer-independent test fixture가 아직 없다.
   - 현재 Character profile은 frontend 내부 상태이며 Unity/외부 renderer로 relay되지 않는다.

4. VRM animation adapter
   - native frontend에는 정적 mesh/rest-skin renderer만 있고 humanoid animation mixer가 없다.
   - Unity 경로에는 WebSocket/TCP/named pipe relay와 Unity companion receiver가 없다.

5. dance asset mapping
   - 현재 Still/Pulse/Bounce/Shake는 sprite/3D viewport 전체에 적용하는 UI effect이며 skeletal animation
     clip이나 expression preset이 아니다.
   - VRMA/dance clip, material/camera cue, critical interrupt policy는 후속 범위다.

## 채택한 MVP와 후속 경로

현재 채택한 MVP는 공통 Character presenter와 native high-spec 정적 VRM viewport다.

1. 기존 deterministic alert state를 `idle`/`warning`/`critical`로 압축한다.
2. 사용자가 상태별 Still/Pulse/Bounce/Shake와 message를 설정한다.
3. native는 SQLite, browser는 tenant/legacy scope의 `localStorage`에 profile을 저장한다.
4. native low/high-spec은 PNG/JPEG를 표시하고 high-spec은 VRM 0.x/1.0도 정적 3D로 표시한다.
5. model path가 비어 있거나 load/validation/GPU 준비가 실패하면 built-in model로 fallback한다.
6. renderer, model 또는 profile 문제가 alert 평가와 low-spec dashboard를 중단시키지 않는다.

표정, SpringBone, MToon 전체 표현과 VRMA dance가 필요하면 native animation adapter를 확장하거나
`.NET extension -> Unity companion -> UniVRM` 경로를 후속 구현해야 한다. Unity 경로는 현재 구현된
것으로 간주하지 않는다.

## 짧은 답

- low/high-spec frontend에서 alert에 반응하는 사용자 설정형 Character panel을 지원한다.
- native에서는 PNG/JPEG/VRM model을 선택하고 상태별 motion/message를 설정할 수 있다.
- native high-spec은 VRM 0.x/1.0 embedded mesh, rest skin, base-color texture를 정적 3D로 표시한다.
- low-spec과 browser는 VRM filesystem path를 렌더링하지 않고 built-in character를 유지한다.
- profile은 native SQLite와 browser `localStorage`에 저장한다.
- OpenTelemetry 신호가 frontend까지 도달하는 경로는 이미 구현되어 있다.
- OpenTelemetry 신호를 외부 .NET extension으로 넘기는 경로도 일부 구현되어 있다.
- alert threshold/severity rule은 고정이며 사용자가 바꾸는 것은 Character 표현 action이다.
- VRM expression/SpringBone/MToon 전체/VRMA와 Unity companion은 아직 없다.
- Still/Pulse/Bounce/Shake는 VRM skeletal animation 또는 dance clip이 아니다.
