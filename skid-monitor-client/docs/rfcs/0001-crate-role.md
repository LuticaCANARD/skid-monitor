# Client RFC 0002: skid-monitor-client Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-monitor-client/docs/rfcs/0002-crate-role.md` |
| Scope | `skid-monitor-client` |
| Decision Type | Client receiver, renderer, extension boundary |

## Abstract

`skid-monitor-client`는 사람이 보는 수신 client다. agent가 보낸 `Signal`을 TCP listener로 받아 console에
요약 렌더링하고, 설정된 경우 .NET extension host로 같은 signal을 전달한다.

## Responsibilities

- `SKID_MONITOR_CLIENT_ADDR`에서 length-prefixed JSON `Signal`을 수신한다.
- metrics, traces, logs를 사람이 읽을 수 있는 console text로 렌더링한다.
- `SKID_MONITOR_EXTENSION_HOST`가 있으면 child process를 띄운다.
- `SKID_MONITOR_DOTNET_EXTENSIONS`가 가리키는 extension assembly가 signal event를 받을 수 있게 한다.

## Runtime Shape

현재 receiver는 blocking `TcpListener`로 한 연결에서 signal 하나를 읽는다. render 후 extension host가
있으면 NDJSON event를 stdin으로 publish한다. extension 실패는 기본 console render를 대체하지 않는다.

## Boundaries

client는 monitor data consumer다. host metric 수집, device socket listen, file transfer, compute
execution은 담당하지 않는다. rich UI, Unity/VRM bridge, WebGPU preview는 extension 또는 future GUI
layer에서 다룬다.

## Non-Goals

- OpenTelemetry SDK collector가 되지 않는다.
- agent/client transport의 server role을 agent로 뒤집는 설계를 여기서 단독으로 확정하지 않는다.
- C# SDK에서 OTLP 전체 모델을 강타입 public API로 고정하지 않는다.

## Open Questions

- multiple agent를 동시에 받을 때 receiver concurrency를 어떻게 둘지.
- stream preview endpoint를 core console에 얼마나 노출할지.
- extension backpressure와 timeout을 client process가 어디까지 강제할지.
