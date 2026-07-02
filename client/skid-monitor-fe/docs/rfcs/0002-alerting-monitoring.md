# 알람 기반 서버 모니터링

## 상태

Draft

## 목표

`skid-monitor-fe`는 agent, file node, extension host에서 들어오는 metrics/logs/traces를 감시하고,
운영자가 조치해야 하는 이상 상태를 알람으로 승격한다.

VRM 아바타는 이 기능의 핵심 로직이 아니라 알람을 사용자에게 전달하는 presenter 중 하나다.
따라서 알람 시스템은 저사양 빌드에서도 동작해야 하며, 고사양 빌드에서는
[0003-vrm-avatar-presenter.md](0003-vrm-avatar-presenter.md)에 정의된 아바타 표현을 추가한다.

## 결정

알람 기능은 다음 4단계로 분리한다.

1. 신호 수집: 기존 `ReceiverMessage::Signal`과 `MetricSample` 변환을 사용한다.
2. rule 평가: metric 이름, source, attribute, numeric value를 기준으로 알람 후보를 만든다.
3. 상태 전이: 같은 문제가 반복될 때 `normal`, `pending`, `firing`, `acknowledged`, `resolved` 상태를 관리한다.
4. presenter 전파: dashboard badge, event log, message, VRM avatar 같은 표현 계층으로 알람 상태를 전달한다.

## 비목표

- 1차 구현에서 Prometheus Alertmanager 수준의 라우팅, silence, escalation policy를 만들지 않는다.
- 1차 구현에서 외부 push notification, SMS, email을 직접 보내지 않는다.
- VRM 렌더링 실패가 알람 평가를 막아서는 안 된다.

## 입력

현재 frontend가 이미 들고 있는 입력을 우선 사용한다.

- `MetricSample.name`: `system.cpu.usage`, `system.memory.usage`, `file_node.root.available` 같은 metric 이름
- `MetricSample.numeric`: threshold 평가 가능한 수치
- `MetricSample.source`: agent, file node, unknown source 구분
- `MetricSample.attributes`: root label, cpu id, service, scope 같은 표시용 context
- `ReceiverMessage::Error`: receiver bind 실패, receive error 같은 frontend 자체 장애
- `ReceiverMessage::ExtensionError`: extension host publish 실패

## 기본 알람 규칙

MVP 규칙은 코드에 내장된 conservative preset으로 시작한다.

| id | 조건 | severity | 설명 |
| --- | --- | --- | --- |
| `receiver.error` | `ReceiverMessage::Error` 발생 | critical | frontend가 신호를 받을 수 없음 |
| `extension.error` | `ReceiverMessage::ExtensionError` 발생 | warning | 외부 extension publish 실패 |
| `system.cpu.high` | `system.cpu.usage >= 90` 이 일정 시간 유지 | warning | CPU saturation 가능성 |
| `system.memory.high` | `system.memory.usage >= 90` 이 일정 시간 유지 | warning | memory pressure 가능성 |
| `file.root.unavailable` | `file_node.root.available == 0` | critical | 등록된 file root 접근 불가 |

이후 사용자 설정 파일로 rule을 확장할 수 있게 한다.

## 상태 모델

알람은 같은 `alert_key` 단위로 집계한다.

`alert_key`는 다음 값을 조합한다.

- rule id
- source
- metric name
- rule이 지정한 주요 attribute subset

상태 전이는 다음과 같다.

```text
normal -> pending -> firing -> acknowledged -> resolved -> normal
               \          \                    /
                \---------- resolved ----------/
```

- `pending`: 조건은 맞지만 아직 지속 시간 조건을 만족하지 않음
- `firing`: 사용자에게 알려야 하는 활성 알람
- `acknowledged`: 사용자가 인지했지만 조건은 아직 해결되지 않음
- `resolved`: 조건이 해소되어 사용자에게 회복을 알려야 하는 상태

## 중복 알림 방지

같은 `alert_key`는 상태가 바뀔 때만 강한 알림을 만든다.

- `pending -> firing`: 새 알람
- `firing -> acknowledged`: 조치 인지
- `firing|acknowledged -> resolved`: 회복
- `resolved -> normal`: 조용히 정리

조건이 계속 유지되는 동안에는 UI 표시만 갱신하고, 메시지와 avatar 표현을 반복 재생하지 않는다.

## UI 요구사항

- header 영역에 현재 최고 severity를 표시한다.
- event log에는 알람 발생, acknowledge, resolved 이벤트가 남아야 한다.
- metrics table에서는 알람에 연관된 metric row를 눈에 띄게 표시한다.
- 저사양 빌드에서는 dashboard badge와 event log만으로도 충분히 상태를 알 수 있어야 한다.
- 고사양 빌드에서는 같은 `AlertSnapshot`을 VRM presenter로 전달한다.

## 데이터 구조 초안

```rust
pub(crate) enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

pub(crate) enum AlertStatus {
    Pending,
    Firing,
    Acknowledged,
    Resolved,
}

pub(crate) struct AlertSnapshot {
    pub(crate) key: String,
    pub(crate) rule_id: String,
    pub(crate) severity: AlertSeverity,
    pub(crate) status: AlertStatus,
    pub(crate) source: String,
    pub(crate) summary: String,
    pub(crate) detail: String,
}
```

메시지 문구 생성은 [0004-alert-message-generation.md](0004-alert-message-generation.md)에서 다룬다.

## 수용 기준

- 샘플 metric이 threshold를 넘으면 `firing` 알람이 생성된다.
- 같은 알람 조건이 유지될 때 새 알림이 무한히 쌓이지 않는다.
- 조건이 해소되면 `resolved` 이벤트가 생성된다.
- VRM presenter가 꺼져 있어도 알람 기능은 정상 동작한다.
- high-spec 빌드에서는 최고 severity가 avatar expression state로 매핑될 수 있다.

## 열린 질문

- 1차 rule preset을 코드에 둘지, `examples/alert-rules.json` 같은 파일로 둘지 결정해야 한다.
- acknowledge와 snooze를 어디까지 MVP에 포함할지 결정해야 한다.
- agent offline 감지는 heartbeat metric을 추가할지, source별 last-seen 기반으로 frontend에서 판단할지 결정해야 한다.
