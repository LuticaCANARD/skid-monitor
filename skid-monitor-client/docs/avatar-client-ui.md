# Avatar Client UI Direction

skid-monitor의 클라이언트 UI는 관측 데이터를 빠르게 읽을 수 있어야 한다. VRM 캐릭터는
대시보드를 대체하는 메인 화면이 아니라, 상태 변화를 더 직관적으로 느끼게 해 주는 표현
레이어로 두는 편이 안정적이다.

권장 방향은 간단하다.

- Tauri UI는 사람이 실제로 판단하는 metrics, logs, traces, alerts 화면을 맡는다.
- VRM/Unity 표현은 현재 상태를 한눈에 알아차리게 하는 companion layer로 둔다.
- 첫 버전은 화려한 캐릭터 시스템보다 `Signal -> 상태 -> 표현` 흐름을 작고 명확하게 만든다.

## Client Experience

클라이언트가 처음 봐야 하는 것은 캐릭터가 아니라 현재 시스템이 안전한지, 어디가 흔들리는지,
방금 어떤 신호가 들어왔는지다. 따라서 첫 화면은 다음 순서로 읽히는 것이 좋다.

1. 전체 상태: normal, warning, critical 같은 짧은 health summary
2. 최근 변화: latency 증가, error log 급증, edge device 온도 상승 같은 주요 이벤트
3. 원인 후보: 관련 service, host, device, trace/log 링크
4. 표현 레이어: VRM 캐릭터의 표정, 자세, 시선, 색조 변화

VRM은 "귀여운 장식"보다 "상태를 놓치지 않게 해 주는 신호 증폭기"에 가깝게 설계한다. 예를
들어 alert가 발생하면 캐릭터가 화면 앞으로 다가오되, 로그 목록이나 경고 버튼을 가리지 않아야
한다. 대시보드 가독성이 항상 우선이다.

## Recommended Architecture

```text
skid-monitor-agent / skid-edge-agent
        |
        v
skid-monitor-client or Tauri backend
        |
        v
Signal event stream
        |
        v
Frontend store
   |             |
Dashboard     Avatar state machine
                 |
                 +-- Three.js VRM in Tauri
                 +-- Unity VRM companion client
```

Rust 쪽은 기존 `Signal` 수신 로직을 재사용한다. Tauri backend는 받은 `Signal`을 frontend event로
emit하고, frontend는 대시보드 상태와 avatar 상태 머신을 분리해서 관리한다. 이 분리는 중요하다.
VRM 로더나 Unity runtime이 잠시 실패해도 대시보드는 계속 살아 있어야 한다.

## State Mapping

초기 상태 매핑은 작게 시작한다. 모델, 애니메이션, 조명 효과가 늘어나도 대시보드와 주고받는
계약은 아래처럼 단순하게 유지한다.

| Signal condition | Client state | Avatar expression |
| --- | --- | --- |
| 정상 상태 | `normal` | idle, 가벼운 breathing |
| latency 증가 | `degraded` | 찡그린 표정, 느린 움직임 |
| error log 폭증 | `noisy` | 당황한 표정, 빠른 시선 이동 |
| edge device 온도 상승 | `thermal_warning` | 땀, 붉은 조명, 더운 제스처 |
| alert 발생 | `critical` | 화면 앞으로 다가오기, 알림 제스처 |

상태 이름은 렌더러 독립적인 값으로 둔다. Tauri의 Three.js VRM renderer와 Unity renderer가 같은
상태 값을 받아 각자 알맞은 표현으로 바꿀 수 있어야 한다.

## Tauri-Friendly Path

Tauri 버전은 skid-monitor의 기본 데스크톱 클라이언트로 적합하다.

- Rust backend가 TCP 수신, protocol decode, local persistence, 설정 관리를 맡는다.
- WebView frontend가 dashboard, filter, timeline, VRM scene을 렌더링한다.
- VRM은 Three.js 기반 로더를 사용하고, `normal/degraded/critical` 같은 상태만 입력받는다.
- GPU 사용량이 높거나 WebGL 호환성이 낮은 환경에서는 VRM layer를 끄고 dashboard만 유지한다.

Tauri MVP는 dashboard-first로 잡는다. VRM scene은 화면 오른쪽 하단, 별도 panel, 혹은 detachable
view처럼 숨기거나 축소할 수 있는 위치가 좋다. 캐릭터가 메인 워크플로를 막으면 모니터링 도구로서
신뢰가 떨어진다.

## Unity-Friendly Path

Unity는 VRM 표현 품질, animation controller, Timeline, Cinemachine, post-processing을 활용하고
싶을 때 좋은 선택지다. 특히 UniVRM을 쓰면 VRM import, blendshape/expression, spring bone 처리가
Tauri/WebView보다 자연스럽다.

Unity 친화적인 구성은 두 가지다.

1. Unity companion client
   - Tauri 또는 Rust client가 `Signal`을 요약한 avatar state event를 내보낸다.
   - Unity 앱은 WebSocket, stdin, named pipe, 또는 localhost TCP로 event를 구독한다.
   - Unity는 UniVRM 모델과 Animator Controller를 사용해 상태를 표현한다.

2. Unity-first visual client
   - Unity가 전체 화면형 관측 뷰를 맡고, Rust/Tauri 쪽은 수신 daemon 또는 설정 UI로 남는다.
   - 관측 표는 uGUI/UI Toolkit으로 구성하되, 복잡한 분석 화면은 Tauri dashboard에 남기는 편이
     유지보수하기 쉽다.

초기에는 Unity를 Tauri WebView 안에 억지로 넣기보다 companion process로 분리하는 편이 좋다.
프로세스가 분리되면 Unity player crash, GPU 부하, asset loading 문제가 Rust client나 dashboard를
같이 죽이지 않는다.

## Unity Event Contract

Unity 쪽에는 raw OTLP 전체를 그대로 넘기기보다, UI가 바로 쓰기 쉬운 얇은 event를 제공하는 것이
좋다.

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

Unity 구현에서는 이 event를 `SkidAvatarState` 같은 C# DTO로 받고, `ScriptableObject` 기반 mapping
asset으로 상태와 animation clip, expression preset, material tint, camera cue를 연결한다. 이렇게
두면 디자이너와 Unity 개발자가 Rust protocol 세부사항을 몰라도 표현을 조정할 수 있다.

이미 있는 .NET extension host도 Unity 친화적인 연결 지점이 될 수 있다. 현재 Rust client는
newline-delimited JSON event를 out-of-process .NET host에 전달할 수 있으므로, 이후 Unity bridge를
C# extension으로 만들면 같은 이벤트를 Unity용 WebSocket 또는 localhost TCP stream으로 relay할 수
있다.

## Risks To Check

- WebView의 WebGL 호환성
- GPU 사용량과 노트북 배터리 영향
- VRM 모델 파일 크기와 cold start 시간
- Linux 환경의 WebView 렌더링 차이
- Unity player 패키징 크기와 자동 업데이트 방식
- Tauri dashboard와 Unity companion process 사이의 lifecycle 관리
- 대시보드 가독성과 캐릭터 연출 사이의 균형

## MVP Scope

초기 버전은 다음 범위로 시작한다.

1. Tauri shell을 만들고 기존 client 수신 흐름을 backend로 옮긴다.
2. `Signal`을 frontend event로 전달한다.
3. frontend store에서 최근 signal, health summary, avatar state를 관리한다.
4. dashboard는 VRM과 독립적으로 기본 metrics/logs/traces를 표시한다.
5. VRM scene은 `normal`, `degraded`, `critical` 정도의 작은 상태만 지원한다.
6. Unity companion client가 붙을 수 있도록 `skid.monitor.avatar.v1` event contract를 문서화한다.

이 범위까지 구현하면, VRM/Unity 표현이 단순 장식인지 실제 관측 경험을 돕는지 빠르게 검증할 수
있다. 검증이 되면 Unity 쪽은 UniVRM asset pipeline, Animator mapping, event replay tool 순서로
확장한다.
