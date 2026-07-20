# VRM 아바타 알람 Presenter

## 상태

Draft

2026-07-20 구현 메모: native `high-spec`의 정적 VRM 0.x/1.0 mesh/rest-skin/base-color renderer와
`idle`/`warning`/`critical` viewport motion은 구현됐다. 이 RFC의 expression, SpringBone, MToon 전체,
VRMA, `Notice`/`Relieved` one-shot 범위는 여전히 Draft다.

## 목표

고사양 frontend 빌드에서 VRM 아바타를 렌더링하고, 알람 상태를 표정, 자세, 색상, 짧은 말풍선으로 표현한다.

이 RFC는 알람 시스템 자체가 아니라 [0002-alerting-monitoring.md](0002-alerting-monitoring.md)의
`AlertSnapshot`을 사용자에게 전달하는 표현 계층을 정의한다.

## 결정

- VRM 아바타는 `high-spec` feature에서만 활성화한다.
- `low-spec` feature에서는 동일한 알람을 dashboard badge, table highlight, event log로 표현한다.
- VRM 렌더링, 모델 로딩, 애니메이션 실패는 알람 상태 평가에 영향을 주지 않는다.
- MVP는 avatar rendering surface와 severity mapping을 먼저 만든다.

## VRM 포맷 판단

VRM은 네트워크 프로토콜이 아니라 `.vrm` 확장자를 쓰는 glTF/GLB 기반 humanoid avatar 파일 포맷이다.

참고:

- https://vrm.dev/en/vrm/vrm_about/
- https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_vrm-1.0
- https://github.com/vrm-c/vrm-specification/tree/master/specification/0.0

VRM 1.0에서 핵심 확장은 다음이다.

- `VRMC_vrm`: humanoid, meta, first person, expressions, lookAt 정의
- `VRMC_materials_mtoon`: toon material
- `VRMC_springBone`: 머리카락, 의상 같은 흔들림 표현
- `VRMC_node_constraint`: node constraint

별도 애니메이션 파일은 `.vrma` 확장자와 `VRMC_vrm_animation` extension을 사용한다.

참고:

- https://vrm.dev/en/vrma/
- https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_vrm_animation-1.0

## MVP 범위

1차 구현은 VRM 전체 호환을 목표로 하지 않는다.

- `.vrm` 파일 선택 경로를 제공하고 일반 `.glb`는 VRM으로 오인하지 않도록 거부한다.
- glTF/GLB loader를 통해 mesh, node hierarchy, skin, texture를 읽을 수 있는 구조를 둔다.
- `VRMC_vrm.meta`와 `VRMC_vrm.humanoid`를 파싱해 avatar 정보와 humanoid bone mapping을 확인한다.
- 알람 severity를 avatar state로 매핑한다.
- MToon, SpringBone, VRMA는 fallback 가능한 후속 단계로 둔다.

현재 runtime은 위 범위 중 mesh, node hierarchy, rest-pose skinning, embedded base-color texture,
meta/humanoid 검증과 severity mapping을 구현했다. MToon 고유 shading, expression morph, SpringBone,
constraint/lookAt, VRMA/glTF animation은 실행하지 않는다.

## 상태 매핑

| 알람 상태 | Avatar state | 표현 |
| --- | --- | --- |
| 알람 없음 | `Idle` | neutral expression, calm idle pose |
| info | `Notice` | small attention gesture, blue accent |
| warning | `Concerned` | concerned expression, yellow accent, short message |
| critical | `Urgent` | urgent expression, red accent, stronger motion |
| resolved | `Relieved` | relief expression, brief recovery message |

아바타 state는 최고 severity의 활성 알람을 기준으로 한다.
여러 알람이 동시에 있으면 `critical > warning > info` 순으로 우선한다.

현재 runtime state는 `Idle`, `Concerned`, `Urgent`만 사용한다. `Notice`, `Relieved`와 expression/pose
mapping은 구현 완료로 간주하지 않는다.

## UI 배치

아바타 viewport는 dashboard의 보조 패널이어야 한다.
운영자가 metric table과 event log를 읽는 흐름을 방해하면 안 된다.

권장 배치:

- wide layout: 오른쪽 또는 상단 보조 panel
- stacked layout: counters 아래, metrics table 위
- compact layout: 접을 수 있는 avatar strip 또는 비활성화

## 렌더링 단계

1. `high-spec` feature에서 `eframe::Renderer::Wgpu`를 사용한다.
2. avatar module은 `viewer3d` 또는 `avatar` namespace 아래에 둔다.
3. egui panel은 알람 상태와 viewport rect만 넘기고, 렌더링 구현 세부사항을 숨긴다.
4. 모델 로딩 실패 시 placeholder state를 표시한다.
5. 렌더링 실패 시 알람 presenter 목록에서 VRM만 비활성화한다.

## Fallback 정책

- VRM 파일이 없으면 built-in placeholder avatar state를 사용한다.
- VRM extension/meta/humanoid 검증이 실패한 일반 GLB나 손상 파일은 표시하지 않고 fallback한다.
- MToon을 구현하지 못한 material은 glTF PBR 또는 unlit fallback을 사용한다.
- SpringBone을 구현하지 않아도 알람 표현은 bounded viewport motion과 말풍선으로 동작해야 한다.

## 데이터 구조 초안

```rust
pub(crate) enum AvatarAlertState {
    Idle,
    Notice,
    Concerned,
    Urgent,
    Relieved,
}

pub(crate) struct AvatarPresenterInput {
    pub(crate) state: AvatarAlertState,
    pub(crate) message: Option<String>,
    pub(crate) active_alert_count: usize,
}
```

`AvatarPresenterInput`은 알람 core가 아니라 presenter adapter에서 만든다.

## 수용 기준

- `high-spec` 빌드에서 avatar viewport를 켜고 끌 수 있다.
- 알람 severity가 바뀌면 avatar state도 바뀐다.
- VRM 파일이 없거나 로딩 실패해도 dashboard는 계속 동작한다.
- `low-spec` 빌드에는 VRM loader와 renderer 의존 경로가 들어가지 않는다.
- critical 알람이 firing 되면 avatar presenter가 `Urgent` 상태를 받을 수 있다.

## 열린 질문

- 기본 bundled avatar asset을 둘지는 모델 재배포 라이선스를 확인한 뒤 결정해야 한다. 현재는 사용자
  파일만 허용한다.
- expression/MToon/SpringBone/VRMA를 native renderer에 넣을지 Unity companion으로 둘지 결정해야 한다.
- browser VRM binary를 tenant별 IndexedDB/OPFS에 영속화할지 결정해야 한다.
- avatar 음성 안내를 넣을지, 화면 메시지만 둘지 결정해야 한다.
