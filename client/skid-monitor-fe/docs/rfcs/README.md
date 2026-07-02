# skid-monitor-fe RFCs

`skid-monitor-fe` crate에만 해당되는 구조적 결정, UI 계약, 고사양/저사양 frontend 경계를 기록한다.

## Index

Status 범례: Draft = 설계 합의 중인 forward-looking 문서로 코드를 권위로 삼지 않으며 구현 정합
검증 대상이 아니다. Accepted = 설계가 고정된 문서. Superseded = 후속 RFC로 대체된 문서.

| RFC | Status | Title | Scope |
| --- | --- | --- | --- |
| [0001](0001_replace_to_egui.md) | Draft | Tauri에서 egui로 변경함 | frontend runtime, egui native app boundary |
| [0002](0002-alerting-monitoring.md) | Draft | 알람 기반 서버 모니터링 | alert core, rule evaluator, state machine, dashboard presenter |
| [0003](0003-vrm-avatar-presenter.md) | Draft | VRM 아바타 알람 Presenter | high-spec VRM avatar rendering, severity mapping, fallback policy |
| [0004](0004-alert-message-generation.md) | Draft | 알람 메시지 생성 | deterministic alert templates, VRM speech bubble text, future AI summary boundary |

## Notes

알람 기능은 RFC 0002를 정준 출처로 삼는다. VRM 아바타는 알람 core가 아니라 presenter이므로 RFC 0003에서
별도로 관리한다. 사용자에게 보여줄 문장, 말풍선, 추후 notification 문구는 RFC 0004에서 관리한다.
