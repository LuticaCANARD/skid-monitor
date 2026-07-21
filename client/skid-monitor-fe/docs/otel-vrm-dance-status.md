# OpenTelemetry 신호 기반 VRM/VTuber 댄스 연동 현황

## 결론

프론트엔드는 고정 alert severity에 반응하는 사용자 설정형 Character presenter를 제공한다. 사용자는
`idle`, `warning`, `critical` 상태마다 `Still`, `Pulse`, `Bounce`, `Shake` motion과 말풍선 message를
설정하고 VRM preset/custom expression 이름을 지정할 수 있다. native `high-spec` 빌드는 사용자 `.vrm`
파일의 VRM 1.0 또는 legacy 0.x 구조를 검증하고 embedded mesh, node hierarchy, MToon 전용 texture와
joint palette GPU skinning을 WGPU 3D viewport에 표시한다. expression morph, pointer lookAt,
roll/aim/rotation constraint와 SpringBone을 VRM 권장 순서로 평가하며, 모델 내부 glTF clip과 최대 8개
외부 `.vrma` 파일의 humanoid clip을 순차 재생하고 경계에서 crossfade한다.

Character panel과 built-in/PNG/JPEG 경로는 `low-spec`과 `high-spec` 모두에서 동작한다. `low-spec`에서
VRM path를 선택하거나 VRM/VRMA 로딩이 실패하면 built-in 2D character로 fallback한다. 현재 animation
player는 node TRS와 VRMA humanoid rotation/hips translation, 다중 clip crossfade를 지원한다. 다만
skeletal clip sequence 자체는 alert state와 무관하며 material/texture-transform expression bind,
상태별 clip 선택, one-shot interrupt와 root-motion policy는 구현하지 않았다.

즉, 지금 바로 있는 것을 기준으로 보면 다음처럼 판단할 수 있다.

| 항목 | 현재 상태 | 근거 |
| --- | --- | --- |
| OpenTelemetry/OTLP 수신 | 구현됨 | `skid-monitor-agent`가 OTLP gRPC metrics/traces/logs receiver를 제공한다. |
| agent에서 client/frontend로 신호 전달 | 구현됨 | `Signal::{Metrics, Traces, Logs}`를 length-prefixed JSON TCP frame으로 보낸다. |
| egui frontend의 신호 표시 | 구현됨 | `skid-monitor-fe`가 metrics/traces/logs를 받아 dashboard, counter, event log로 표시한다. |
| high-spec VRM renderer | Prototype | `high-spec` feature가 WGPU depth viewport에서 native VRM 0.x/1.0 mesh, texture, MToon 핵심 shading과 GPU skinning을 표시한다. |
| 2D Character presenter | 구현됨 | low/high-spec 공통 Character panel이 built-in 또는 native PNG/JPEG 모델을 표시한다. |
| 사용자 reaction profile | 구현됨 | `idle`/`warning`/`critical`별 Still/Pulse/Bounce/Shake, custom message와 VRM expression을 설정한다. |
| Character profile 영속화 | 구현됨 | native는 SQLite, browser는 인증된 cloud tenant scope 또는 legacy local scope의 `localStorage` key에 profile을 저장한다. |
| 알람 -> VRM 상태 설계 | 문서화됨 | RFC 0002/0004에 `AlertSnapshot`, short message, VRM presenter 전달이 정의되어 있다. |
| MToon | 부분 구현 | 1.0/legacy의 shade/shift/toony, GI 근사, rim/matcap, emission factor, outline, UV scroll/rotation, transparent Z-write/render queue와 shade/normal/matcap/rim/outline-width 전용 texture를 반영한다. shading-shift 및 UV-animation mask texture는 미지원이다. |
| VRM runtime | 부분 구현 | VRM 0.x/1.0 expression morph와 자동 blink, bone/expression pointer lookAt, VRMC roll/aim/rotation constraint, SpringBone sphere/capsule/center를 실행한다. material/texture-transform expression bind는 없다. |
| VRMA/skeletal animation | 부분 구현 | GPU skinning, 모든 embedded glTF clip과 최대 8개 외부 VRMA 파일의 clip FK 리타기팅, STEP/LINEAR/CUBICSPLINE 반복·crossfade를 지원한다. alert-state clip 선택과 root-motion policy는 없다. |
| custom WGSL material | 구현됨 | native high-spec에서 64 KiB 이하 `.wgsl`의 `skid_custom_material` hook을 고정 MToon ABI에 합성한다. global resource/entry point/loop를 거부하고 Naga 검증 실패 시 기본 MToon으로 fallback한다. |
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

각 상태에는 사용자가 지정한 말풍선 message와 VRM expression을 함께 적용할 수 있다. motion은 2D
sprite 또는 VRM viewport를 안전하게 이동하거나 크기 변화시키는 bounded UI effect다. skeletal clip
선택과는 별도이며, 설정한 VRMA/embedded clip sequence는 현재 alert 상태와 무관하게 반복 재생한다.

- native frontend: `.png`, `.jpg`, `.jpeg`, `.vrm` model path와 최대 8개의 optional `.vrma` path 및 `.wgsl` material hook을 선택할
  수 있고 profile은 SQLite write ACK 이후 적용한다. model/animation decode는 크기를 제한한 단일
  background loader에서 직렬 처리한다.
- native `high-spec`: `.vrm`을 GLB로 파싱한다. VRM 1.0은 필수 `meta.name`/`authors`/`licenseUrl`과
  humanoid bone을, legacy 0.0은 별도 필수 bone 집합과 meta object의 알려진 field type을 검증한다.
  external buffer/image URI와 unsupported required extension은 거부한다. node transform, embedded
  texture, expression morph delta, 원본 vertex joint/weight와 동적 pose matrix를 GPU에 올린다. `.vrma`는
  `VRMC_vrm_animation` 1.0과 humanoid hierarchy를 검증한 뒤 target VRM bone에 FK 리타기팅한다.
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

`Character` 창에서 `.vrm`, optional `.vrma` path 목록과 `.wgsl` material hook을 입력하거나 native window에
파일을 drop한 뒤 `Apply & preview character`를 누른다. 저장이 시작되면 별도 `Character preview`가 즉시
열리고, 로딩 중 표시를 거쳐 VRM version과 3D viewport를 보여준다. 저장된 custom model이 있으면 다음
앱 실행 때도 preview를 자동으로 연다. loader는 VRM 128 MiB, VRMA 파일당 64 MiB로 제한하고,
node/primitive/triangle/texture/keyframe 및 decoded texture allocation에도 상한을 둔다. CPU parsing은
기존 single background loader에서 실행하고, GPU resource 생성과 pose buffer 갱신은 egui WGPU callback의
prepare 단계에서만 수행한다. stale generation은 설치하지 않는다.

custom shader는 [예제](../examples/custom-material.wgsl)의 `skid_custom_material` 함수만 제공한다.
기본 MToon의 linear RGBA, normal, UV, world position, time을 받아 최종 linear RGBA를 반환한다. loader는
64 KiB/UTF-8 제한, global variable/resource, entry point와 loop 금지, 독립 및 합성 Naga validation을
검사한다. 실패한 custom shader는 모델 로딩을 취소하지 않고 기본 MToon을 유지하며 UI에 오류를 표시한다.

MToon 1.0과 legacy MToon에서 shade color, shading shift/toony, GI equalization 근사, parametric rim,
matcap, emission factor, inverse-hull outline, UV scroll/rotation, transparent Z-write와 render queue offset을
읽어 전용 WGSL path에 반영한다. base/shade/matcap/rim은 sRGB view, normal과 outline-width mask는 linear
view로 분리한다. outline mask는 VRM 1.0 사양의 G channel과 legacy UniVRM MToon의 R channel을
버전별로 선택한다. shading-shift texture와 UV-animation mask texture는 아직 sampling하지 않는다.

animation player는 모든 glTF STEP/LINEAR/CUBICSPLINE translation/rotation/scale clip을 평가한다. 외부
VRMA는 humanoid rotation과 hips translation을 target rest transform에 FK로 옮기고 신장 비율로 hips
translation을 보정한다. 여러 clip은 순차 loop하며 설정한 시간 동안 다음 clip과 TRS crossfade한다.
expression morph는 별도 VRM expression runtime에서 GPU morph weight로 합산한다. alert-state clip
switching, one-shot interrupt와 root-motion policy는 없다. `.glb` 일반 모델은 VRM으로 가장하지 않도록
profile validation에서 허용하지 않는다.

frame 적용 순서는 VRM 권장 순서에 맞춰 animation/humanoid pose, lookAt, expression, node constraint,
SpringBone이다. SpringBone은 Verlet 적분, sphere/capsule collision과 center space를 지원하고 pointer
lookAt은 bone 또는 expression RangeMap을 사용한다. expression의 material-color/texture-transform bind는
현재 morph path에 포함하지 않는다. legacy VRM 0.x `secondaryAnimation`은 sphere collider와 각 root의
첫 번째 child chain을 SpringBone runtime으로 변환하는 호환 경로이며, branch별 virtual terminal을
복원하는 완전한 legacy solver는 아니다.

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

현재 high-spec renderer는 VRMA/embedded skeletal clip을 반복 재생할 수 있다. OpenTelemetry 신호가
clip 선택과 전환을 제어하는 VRM/VTuber dance runtime까지 가려면 아래 항목이 추가되어야 한다.

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
   - native frontend에는 다중 clip sequence, FK retargeter와 crossfade가 있지만 alert-state selector,
     one-shot interrupt와 root-motion 정책이 없다.
   - Unity 경로에는 WebSocket/TCP/named pipe relay와 Unity companion receiver가 없다.

5. dance asset mapping
   - Still/Pulse/Bounce/Shake는 sprite/3D viewport 전체에 적용하는 UI effect이며 VRMA clip selector가
     아니다.
   - idle/warning/critical별 VRMA 선택, material/camera cue, critical interrupt policy는 후속 범위다.

## 채택한 MVP와 후속 경로

현재 채택한 MVP는 공통 Character presenter와 native high-spec VRM/MToon/runtime/multi-clip viewport다.

1. 기존 deterministic alert state를 `idle`/`warning`/`critical`로 압축한다.
2. 사용자가 상태별 Still/Pulse/Bounce/Shake, message와 VRM expression을 설정한다.
3. native는 SQLite, browser는 tenant/legacy scope의 `localStorage`에 profile을 저장한다.
4. native low/high-spec은 PNG/JPEG를 표시하고 high-spec은 VRM 0.x/1.0 runtime과 다중 VRMA/embedded clip을 3D로 표시한다.
5. model path가 비어 있거나 load/validation/GPU 준비가 실패하면 built-in model로 fallback한다.
6. renderer, model 또는 profile 문제가 alert 평가와 low-spec dashboard를 중단시키지 않는다.

material/texture-transform expression, 상태 기반 dance 전환과 복합 연출이 필요하면 native adapter를
확장하거나 `.NET extension -> Unity companion -> UniVRM` 경로를 후속 구현해야 한다. Unity 경로는
현재 구현된 것으로 간주하지 않는다.

## 짧은 답

- low/high-spec frontend에서 alert에 반응하는 사용자 설정형 Character panel을 지원한다.
- native에서는 PNG/JPEG/VRM model, 최대 8개 optional VRMA와 상태별 viewport motion/message/expression을 설정할 수 있다.
- native high-spec은 VRM 0.x/1.0 mesh/texture, MToon 전용 map, GPU skinning, expression/SpringBone/lookAt/constraint와 다중 clip crossfade를 표시한다.
- low-spec과 browser는 VRM filesystem path를 렌더링하지 않고 built-in character를 유지한다.
- profile은 native SQLite와 browser `localStorage`에 저장한다.
- OpenTelemetry 신호가 frontend까지 도달하는 경로는 이미 구현되어 있다.
- OpenTelemetry 신호를 외부 .NET extension으로 넘기는 경로도 일부 구현되어 있다.
- alert threshold/severity rule은 고정이며 사용자가 바꾸는 것은 Character 표현 action이다.
- material/texture-transform expression bind, alert-state clip selector와 Unity companion은 아직 없다.
- Still/Pulse/Bounce/Shake는 VRM skeletal clip 선택이 아니라 viewport motion이다.
