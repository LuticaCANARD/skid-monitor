# RFC 0001: skid-edge-agent Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-edge-agent/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-edge-agent` |
| Decision Type | Edge physical signal node responsibility |

## Abstract

`skid-edge-agent`는 edge 장비 주변의 물리/환경 신호를 agent device ingress로 보내는 작은 binary다.
camera/image/video provider처럼 현장 장비 가까이에 붙어야 하는 media provider adapter도 이 crate의
확장 책임으로 둔다. 현재 구현은 실제 센서 대신 deterministic mock sample을 보낸다.

## Responsibilities

- `edge.temperature`, `edge.voltage.input`, `edge.network.rssi` 같은 edge metric을 만든다.
- `device_id`, `node_name`, `sensor` attribute를 붙인다.
- `Source::EdgeDevice`로 OTLP metrics request를 생성한다.
- future media provider mode에서는 `stream.provider.*`, `stream.endpoint.*`, `stream.snapshot.*`
  metadata를 `Source::Stream`으로 보낼 수 있다.
- `SKID_MONITOR_DEVICE_ADDR`로 agent device socket에 length-prefixed JSON `Signal::Metrics`를 보낸다.
- `--once`로 한 번만 보내는 개발 모드를 지원한다.

## Boundaries

이 crate는 edge sensor adapter의 첫 표면이다. 실제 GPIO, I2C, serial, MCU protocol 읽기는 아직 없다.
hardware-specific code는 future sensor layer로 분리하고, protocol 전송 경계는 작게 유지한다. camera나
snapshot provider를 붙이더라도 raw JPEG/PNG/video/audio bytes는 device ingress `Signal`에 싣지 않고,
provider endpoint와 health metadata만 보낸다.

## Non-Goals

- Kubernetes node agent가 아니다.
- agent device socket을 listen하지 않는다.
- file, compute payload를 만들지 않는다.
- media provider를 관측할 수는 있지만, media payload를 device ingress로 운반하지 않는다.

## Open Questions

- 실제 sensor backend를 feature flag로 둘지 별도 adapter crate로 둘지.
- `edge.boot.count`와 `edge.watchdog.resets`의 누적 의미를 실제 장비에서 어떻게 보장할지.
- device enrollment가 들어올 때 `device_id`를 credential과 어떻게 묶을지.
- media provider adapter를 edge sensor adapter와 같은 config namespace에 둘지 별도 `media_providers`
  namespace로 둘지.
