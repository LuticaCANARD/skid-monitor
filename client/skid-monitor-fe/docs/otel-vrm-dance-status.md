# OpenTelemetry 신호 기반 VRM/VTuber 댄스 연동 현황

## 결론

프론트엔드에 VRM/VTuber 캐릭터를 띄우고 OpenTelemetry 신호에 맞춰 춤추게 하는 것은 가능하다.
다만 현재 repository의 실제 구현 수준은 "신호 수신과 외부 확장 전달 경로는 있음"이고,
"VRM 렌더링, 댄스 애니메이션, 신호에서 댄스 상태를 만드는 로직"은 아직 구현 전이다.

즉, 지금 바로 있는 것을 기준으로 보면 다음처럼 판단할 수 있다.

| 항목 | 현재 상태 | 근거 |
| --- | --- | --- |
| OpenTelemetry/OTLP 수신 | 구현됨 | `skid-monitor-agent`가 OTLP gRPC metrics/traces/logs receiver를 제공한다. |
| agent에서 client/frontend로 신호 전달 | 구현됨 | `Signal::{Metrics, Traces, Logs}`를 length-prefixed JSON TCP frame으로 보낸다. |
| egui frontend의 신호 표시 | 구현됨 | `skid-monitor-fe`가 metrics/traces/logs를 받아 dashboard, counter, event log로 표시한다. |
| high-spec 렌더러 선택 | 부분 구현 | `high-spec` feature가 `eframe::Renderer::Wgpu`를 선택한다. |
| VRM presenter 설계 | 문서화됨 | RFC 0003에 VRM avatar presenter, severity mapping, fallback 정책이 있다. |
| 알람 -> VRM 상태 설계 | 문서화됨 | RFC 0002/0004에 `AlertSnapshot`, short message, VRM presenter 전달이 정의되어 있다. |
| 실제 VRM loader/renderer | 미구현 | `avatar`, `viewer3d`, `gltf`, `vrm`, `vrma` 런타임 코드/의존성이 없다. |
| 신호 -> 댄스 상태 변환 | 미구현 | 현재 frontend는 counters/events/metrics만 갱신하고 avatar state machine은 없다. |
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
```

이 흐름은 "관측 신호를 UI까지 가져오는 경로"로는 이미 쓸 수 있다. OpenTelemetry application이
agent의 OTLP endpoint로 export하면, agent pipeline이 `skid` exporter를 통해 frontend/client가
듣는 주소로 `Signal`을 보낸다.

주의할 점은 frontend가 OTLP receiver 자체는 아니라는 것이다. frontend는 `SKID_MONITOR_CLIENT_ADDR`
로 들어오는 Skid `Signal` TCP frame을 받는다. OTLP는 agent가 받는다.

## VRM/VTuber와 소통할 수 있는 기존 연결점

현재 VRM과 직접 소통하는 코드가 있는 것은 아니다. 하지만 외부 VRM 런타임과 연결하기 위한 후보
경로는 이미 일부 있다.

### 1. Native frontend 안에 VRM renderer를 넣는 경로

`skid-monitor-fe`는 현재 egui native app이고, `high-spec` feature에서 wgpu renderer를 선택할 수
있다.

```sh
cargo run -p skid-monitor-fe --no-default-features --features high-spec
```

RFC 0003은 이 경로를 기준으로 다음을 제안한다.

- `high-spec` build에서만 VRM avatar viewport를 켠다.
- `low-spec` build에서는 dashboard badge, table highlight, event log로 fallback한다.
- `.vrm`/`.glb` loader, humanoid bone mapping, expression mapping을 avatar module에 둔다.
- VRM 실패가 dashboard나 alert 평가를 망가뜨리지 않게 분리한다.

현재 코드에는 아직 `avatar` module, VRM/glTF loader, animation mixer, dance clip player가 없다.
따라서 이 경로는 설계는 있지만 구현은 시작 전이다.

### 2. .NET extension host를 Unity/VRM bridge로 쓰는 경로

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

예시 mapping:

| OpenTelemetry 조건 | Avatar state | 표현 |
| --- | --- | --- |
| 정상 heartbeat/metric 수신 | `idle` | idle, breathing, 작은 loop motion |
| metric 유입량 증가 | `active` | 고개 끄덕임, 가벼운 step |
| latency 증가 | `degraded` | 느린 움직임, 걱정 표정 |
| warning alert firing | `warning` | 짧은 주의 gesture |
| critical alert firing | `critical` | 강한 알림 motion, dance 중단 또는 긴급 pose |
| 회복 이벤트 | `relieved` | 안도 gesture, 짧은 recovery motion |
| 특정 metric beat가 안정적으로 들어옴 | `dance_loop` | BPM/강도에 맞춘 dance loop |

댄스는 알람 표현과 충돌하지 않게 우선순위를 둬야 한다.

1. `critical`: 춤보다 경고 pose/gesture가 우선이다.
2. `warning`: dance loop를 약하게 줄이거나 attention gesture를 섞는다.
3. `normal/active`: metric rhythm에 맞춘 dance loop를 재생할 수 있다.
4. `resolved`: 짧은 회복 motion 뒤 idle/dance로 돌아간다.

## 현재 코드 기준 구현 공백

OpenTelemetry 신호가 VRM까지 가려면 아래 항목이 추가되어야 한다.

1. `Signal` 요약기
   - metrics/traces/logs를 avatar가 쓰기 쉬운 작은 상태로 압축한다.
   - 예: `severity`, `source`, `state`, `title`, `attributes`, `intensity`.

2. alert/health state machine
   - RFC 0002의 `AlertSnapshot` 또는 더 작은 `AvatarState`를 실제 코드로 만든다.
   - 같은 조건이 유지될 때 animation trigger가 반복 폭주하지 않게 dedupe한다.

3. avatar event contract
   - `skid.monitor.avatar.v1` JSON schema를 실제 코드와 test fixture로 고정한다.
   - renderer가 Rust/wgpu인지 Unity/UniVRM인지와 무관하게 같은 event를 받게 한다.

4. renderer adapter
   - native frontend 경로: VRM/glTF loader, humanoid mapping, animation mixer, egui/wgpu surface 통합.
   - Unity 경로: C# extension에서 WebSocket/TCP/named pipe로 Unity companion에 event relay.

5. dance asset mapping
   - `AvatarState`를 animation clip, expression preset, material tint, camera cue로 연결한다.
   - critical/warning이 dance loop보다 높은 우선순위를 갖게 한다.

## 추천 MVP

현재 repository 상태에서는 Unity/UniVRM companion 경로가 가장 빠른 MVP다. 이유는 UniVRM이 VRM
import, expression, spring bone, Animator Controller를 이미 잘 처리하고, Rust egui/wgpu 안에
직접 VRM runtime을 넣는 것보다 위험이 작기 때문이다.

추천 순서:

1. C# sample extension을 signal counter에서 signal classifier로 확장한다.
2. classifier가 `skid.monitor.avatar.v1` 이벤트를 생성하게 한다.
3. 이벤트를 stdout이 아니라 localhost WebSocket 또는 TCP로 relay한다.
4. Unity companion client가 event를 받아 `state -> Animator trigger/float/bool`로 매핑한다.
5. 정상 상태에서는 dance loop를 재생하고, warning/critical에서는 dance를 interrupt한다.
6. native frontend VRM renderer는 별도 후속 단계로 두고, `high-spec` feature의 장기 목표로 유지한다.

## 짧은 답

- 프론트엔드에서 VTuber/VRM 캐릭터가 나와 춤추는 것은 가능하다.
- OpenTelemetry 신호가 frontend까지 도달하는 경로는 이미 구현되어 있다.
- OpenTelemetry 신호를 외부 .NET extension으로 넘기는 경로도 일부 구현되어 있다.
- 하지만 OpenTelemetry 신호를 VRM 상태/댄스 애니메이션으로 바꾸는 로직은 아직 없다.
- 실제 VRM 렌더링/UniVRM/Three.js/wgpu avatar runtime도 아직 없다.
- 가장 가까운 구현 경로는 `.NET extension -> Unity companion -> UniVRM` bridge다.
