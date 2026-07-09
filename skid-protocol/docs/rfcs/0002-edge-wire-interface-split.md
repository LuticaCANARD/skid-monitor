# RFC 0002: Edge Wire Interface Split

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-07-09 |
| File | `skid-protocol/docs/rfcs/0002-edge-wire-interface-split.md` |
| Scope | `skid-protocol`, `skid-edge-wire`, `skid-monitor-agent` |
| Decision Type | Embedded-compatible protocol boundary |

## Abstract

MCU/RTOS 장비는 `Signal::Metrics` JSON payload와 OTLP protobuf model을 직접 만들지 않는다.
대신 `skid-edge-wire`의 compact metric frame을 만들고, host-side agent나 gateway가 그 frame을
canonical `Signal::Metrics` / OTLP metrics request로 승격한다.

## Decision

- `skid-protocol`은 agent/client 내부 canonical contract로 유지한다.
- `skid-edge-wire`는 `no_std` crate로 분리하고 allocation, socket, clock, JSON, OTLP 의존성을 갖지 않는다.
- device ingress는 기존 length-prefixed JSON `Signal` payload와 `skid-edge-wire` payload를 함께 받는다.
- `skid-edge-wire` frame은 metric id, sensor id, metric kind, unit, `f32` value, device identity를 담는다.
- metric name, sensor label, OTLP resource/data point attribute 구성은 host-side adapter가 담당한다.

## Boundaries

`skid-edge-wire`는 bytes encode/decode만 담당한다. TCP, UART, CAN, BLE, retry, enrollment credential,
clock source, backoff, OTA update는 firmware나 gateway layer의 책임이다.

`skid-protocol`은 여전히 OTLP `Signal`과 legacy JSON frame helper를 제공한다. embedded payload를 위해
OTLP/JSON model을 `no_std`로 억지 이식하지 않는다.

## Migration

1. Existing `skid-edge-agent` and device clients can keep sending legacy JSON `Signal` frames.
2. MCU firmware can start with `skid-edge-wire::encode_metrics_frame` and send the compact payload behind the same
   device socket length prefix.
3. A later serial/CAN gateway can decode the compact frame and forward either compact payload or canonical `Signal`.

## Open Questions

- device enrollment이 들어오면 numeric `device_id` dictionary를 추가할지.
- `f32` 외에 scaled `i32` value type을 추가할지.
- stream/media provider metadata도 edge-wire에 넣을지, host gateway-only extension으로 둘지.
