# RFC 0001: skid-monitor-agent Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-monitor-agent/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-monitor-agent` |
| Decision Type | Collector and gateway responsibility |

## Abstract

`skid-monitor-agent`는 Skid Monitor의 collector/gateway binary다. 자체 OpenTelemetry 신호와 OS별
host/system metrics를 수집하고, device/OTLP ingress로 들어온 `Signal`을 설정된 agent pipeline으로
fan-out 한다.

## Responsibilities

- 15초 주기로 metrics, traces, logs를 수집해 `Signal`로 전송한다.
- Linux host/system metric을 `/proc` 계열 interface에서 읽어 `skid-protocol`의 metric helper로 OTLP
  request에 합친다.
- macOS host metric을 별도 native sampler로 수집하고 `skid_monitor.source=macos`로 분리한다.
- Windows native host metric은 planned target으로 두며, 구현 시 PDH/WMI/CIM/ETW/Event Log 계열
  adapter와 별도 source 계약을 추가한다.
- `SKID_MONITOR_DEVICE_LISTEN_ADDR`에서 device ingress를 연다.
- 선택적으로 `SKID_MONITOR_OTLP_GRPC_ADDR` 또는 config의 `receivers.otlp.grpc_addr`에서 OTLP gRPC
  receiver를 연다.
- edge/file/compute node가 보낸 length-prefixed JSON `Signal`을 decode하고 pipeline에 투입한다.
- `SKID_MONITOR_AGENT_CONFIG`의 exporter/pipeline 설정에 따라 `skid_client`, `logging`, `otlp`
  exporter로 신호를 fan-out 한다.
- 설정이 없으면 `SKID_MONITOR_CLIENT_ADDR`가 있을 때 client에 TCP connect로 신호를 보낸다.

## Runtime Shape

`main.rs`는 config를 읽고 telemetry guard를 초기화한 뒤 self-observation interval, device socket task,
OTLP gRPC receiver task를 설정에 따라 함께 돌린다. self-observation은 OS별 `SystemSampler` branch에서
host metric을 수집한다. 각 receiver는 `SignalPipeline`으로 신호를 넘기고, pipeline은 signal type별
receiver filter, processor, exporter fan-out을 적용한다. device ingress는 tokio `TcpListener`, OTLP
ingress는 tonic gRPC server를 쓴다. `skid_client` exporter의 client forward path는 현재 blocking TCP
send다.

## Boundaries

agent는 관측 gateway다. file download, compute execution, media relay, client UI rendering, package
manager, binary updater는 담당하지 않는다. device socket으로 받은 신호를 신뢰하기 전에
authentication, read timeout, connection cap, rate limit이 필요하다.

## Non-Goals

- client가 보여줄 UI 정책을 결정하지 않는다.
- capability node를 child process로 관리하지 않는다.
- remote execution scheduler가 되지 않는다.

## Open Questions

- client transport를 client-subscribe 모델로 뒤집을 시점.
- device ingress의 SKDM v1 auto decoder와 public bind guard를 어디까지 agent에 넣을지.
- blocking client send를 tokio I/O로 바꿀지 `spawn_blocking`으로 격리할지.
- Windows native sampler와 `Source` 계약을 어느 RFC에서 고정할지.
- Linux package, macOS `.pkg`, Windows `.msi` 설치 UX를 같은 release train으로 묶을지.
