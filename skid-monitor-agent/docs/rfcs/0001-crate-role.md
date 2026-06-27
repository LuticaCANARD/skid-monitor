# RFC 0001: skid-monitor-agent Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-monitor-agent/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-monitor-agent` |
| Decision Type | Collector and gateway responsibility |

## Abstract

`skid-monitor-agent`는 Skid Monitor의 collector/gateway binary다. 자체 OpenTelemetry 신호와 Linux
host/system metrics를 수집하고, device ingress로 들어온 capability node의 `Signal`을 client로
forward한다.

## Responsibilities

- 15초 주기로 metrics, traces, logs를 수집해 `Signal`로 전송한다.
- Linux host/system metric을 `skid-protocol`의 metric helper로 OTLP request에 합친다.
- `SKID_MONITOR_DEVICE_LISTEN_ADDR`에서 device ingress를 연다.
- edge/file/compute node가 보낸 length-prefixed JSON `Signal`을 decode하고 client로 forward한다.
- `SKID_MONITOR_CLIENT_ADDR`가 있으면 client에 TCP connect로 신호를 보낸다.

## Runtime Shape

`main.rs`는 telemetry guard를 초기화하고 device socket task와 수집 interval loop를 함께 돌린다.
device ingress는 tokio `TcpListener`를 쓰지만 client forward path는 현재 blocking TCP send다.

## Boundaries

agent는 관측 gateway다. file download, compute execution, media relay, client UI rendering은 담당하지
않는다. device socket으로 받은 신호를 신뢰하기 전에 authentication, read timeout, connection cap,
rate limit이 필요하다.

## Non-Goals

- client가 보여줄 UI 정책을 결정하지 않는다.
- capability node를 child process로 관리하지 않는다.
- remote execution scheduler가 되지 않는다.

## Open Questions

- client transport를 client-subscribe 모델로 뒤집을 시점.
- device ingress의 SKDM v1 auto decoder와 public bind guard를 어디까지 agent에 넣을지.
- blocking client send를 tokio I/O로 바꿀지 `spawn_blocking`으로 격리할지.
