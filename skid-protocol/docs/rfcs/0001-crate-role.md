# RFC 0001: skid-protocol Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-protocol/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-protocol` |
| Decision Type | Shared protocol crate responsibility |

## Abstract

`skid-protocol`은 workspace의 공통 wire/data 계약을 담는 library crate다. agent, client, edge/file/
compute node가 같은 `Signal` enum과 OTLP export request 타입을 공유하게 만든다.

## Responsibilities

- `Signal::Metrics`, `Signal::Traces`, `Signal::Logs`를 정의한다.
- OTLP protobuf export request 타입을 crate 외부에 재노출한다.
- SDK aggregator를 거치지 않는 간단 metric 표본을 OTLP metrics request로 변환한다.
- `Source`와 `MetricKind`의 정준 Rust 타입을 제공한다.
- 현재 legacy TCP wire contract인 length-prefixed JSON `Signal` frame helper를 제공한다.
- TCP, filesystem, tokio runtime, OpenTelemetry SDK exporter를 직접 소유하지 않는다.

## Boundaries

이 crate는 runtime이 아니다. 수집 주기, socket listen/connect, 화면 렌더링, extension host 실행은
각 binary crate가 담당한다. `skid-protocol`은 직렬화 가능한 계약과 변환 helper만 가진다.

## Current API Surface

- `protocol.rs`: agent/client/device socket이 주고받는 `Signal`
- `frame.rs`: legacy length-prefixed JSON `Signal` encoder/decoder
- `metrics.rs`: `Metric`, `Source`, `MetricKind`, `export_metrics`
- `otlp.rs`: OpenTelemetry protobuf 타입 재노출

## Non-Goals

- socket listen/connect 같은 transport runtime을 소유하지 않는다.
- OpenTelemetry SDK provider나 exporter를 구성하지 않는다.
- client 표시 정책이나 agent 수집 정책을 포함하지 않는다.

## Open Questions

- future SKDM v1 frame parser/writer를 이 crate에 둘지, 별도 transport crate로 나눌지.
- `Source::Stream`을 언제 실제 enum 변형으로 추가할지.
- `Signal` 자체에 protocol version과 node identity envelope를 넣을지.
