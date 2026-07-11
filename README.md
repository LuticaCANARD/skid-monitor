# Skid Monitor

Skid Monitor는 애플리케이션, 호스트, edge 장비, 파일 접근 노드, 분산 compute capability에서
나오는 신호를 하나의 가벼운 프로토콜로 모으는 실험적 모니터링/라우팅 툴킷이다.

이 repository는 SKID 계열 실험의 통합 지점이다. 다른 `skid-*` repository에서 확인한 요소는
먼저 RFC로 옮겨 설계를 고정하고, 실제 구현은 이 repository 안에서만 진행한다.

현재 워크스페이스는 여러 Rust crate와 client-side .NET binding 프로젝트로 나뉜다.

- `skid-protocol`: agent/client/edge adapter가 공유하는 OTLP 기반 직렬화 계약
- `skid-monitor-core`: Solo/Cloud가 공유하는 signal envelope, cursor, projection/store 계약
- `skid-monitor-server`: PostgreSQL/Keycloak 기반 cloud agent-ingress와 client-access 서버
- `skid-monitor-agent`: OpenTelemetry 및 OS별 host/system 신호를 수집해 `Signal`로 전송하는 agent
- `skid-edge-agent`: edge 물리 계층/환경 신호를 수집해 `Signal`로 전송하는 agent
- `skid-file-node`: read-only file offer 후보 root를 관측 신호로 알리는 초기 file node
- `skid-compute-advisor`: 병렬 처리 capability와 route advice 후보를 알리는 초기 compute advisor
- `skid-monitor-client`: `Signal`을 받아 콘솔에 표시하고 C# 확장 호스트로 전달하는 client
- `skid-monitor-fe`: egui 기반 control-room desktop frontend
- `client/skid-monitor-client/bindings/dotnet/`: 별도 SDK 라이브러리, out-of-process .NET extension host, sample extension

배포 경계, 설정/transport, device ingress frame, compute advisor, stream telemetry의 초기 결정은
[RFC 0001: Initial Skid Monitor Integration](docs/rfcs/0001-initial-skid-monitor-integration.md)를 따른다.
Kubernetes/Talos 배포는 [docs/deployment.md](docs/deployment.md), Linux/macOS/Windows native agent의
지속 배포와 자동 업데이트 정책은 [docs/agent-continuous-deployment.md](docs/agent-continuous-deployment.md)를
따른다. 한 사용자용 trusted-local Solo 실행과 PostgreSQL/Keycloak 기반 분리 cloud 실행은
[docs/cloud-solo-deployment.md](docs/cloud-solo-deployment.md)에 정리한다.
PostgreSQL 컴포넌트, 권한 분리와 migration 운영 절차는
[docs/postgresql-components-and-migrations.md](docs/postgresql-components-and-migrations.md)에 정의한다.
클라이언트 UI와 C# extension 개발 방향은 [skid-monitor-client/docs](skid-monitor-client/docs/README.md)에 둔다.

다른 SKID 계열 repository에서 가져온 설계 후보도 먼저 RFC 0001에 통합해 정준 계약을 고정한 뒤,
필요할 때 후속 RFC로 분리한다.

각 Rust crate의 역할 RFC와 사용 use case는 해당 crate 아래의 `docs/rfcs`와 `docs/usecases`에 둔다.

## Frontend Control Room

`client/skid-monitor-fe`는 egui 기반 native desktop frontend다. Tauri/WebView 대신 Rust UI로
agent/client TCP signal을 직접 수신해 control-room dashboard로 보여준다. VRM/아바타 같은 3D 표현은
Unity binding 쪽 책임으로 두고, 이 frontend는 운영자가 실시간 metric/log/trace 상태를 빠르게 훑는
관제 화면에 집중한다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-fe
```

다른 터미널에서 같은 주소로 agent를 실행한다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

여러 노드의 agent를 노드별 local port-forward나 overlay endpoint로 받을 때는 FE가 여러 listen
주소를 동시에 연다. `SKID_MONITOR_CLIENT_ADDRS`는 FE/client 전용 comma-separated 목록이고,
각 agent는 자기 노드에 대응하는 단일 `SKID_MONITOR_CLIENT_ADDR`로 보낸다.
FE는 한 창 안의 `Nodes` 표에서 node/endpoint/source/service와 최신 signal 값을 행 단위로 보여준다.

```sh
SKID_MONITOR_CLIENT_ADDRS=127.0.0.1:9000,127.0.0.1:9001 cargo run -p skid-monitor-fe

# node-a agent/exporter
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent

# node-b agent/exporter
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9001 cargo run -p skid-monitor-agent
```

## Host Metrics

`skid-monitor-agent`는 OpenTelemetry 자체 계측뿐 아니라 OS별 호스트 메트릭도 함께 수집한다.
Linux에서는 CPU 사용률, load average, 메모리/스왑, uptime, 파일시스템 용량, 디스크 I/O,
네트워크 I/O, agent 프로세스 메모리/스레드/open FD 상태를 `/proc` 계열 interface에서 읽는다.

모든 값은 `Signal::Metrics(ExportMetricsServiceRequest)`로 client에 전송된다. Linux host 메트릭은
OTLP resource attribute `skid_monitor.source=system`으로 표시된다.

```sh
# terminal 1: 사람이 보는 client
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-client

# terminal 2: 서버 메트릭 수집 agent
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

예전 `MONITOR_CAT_*` 환경변수도 당분간 fallback으로 읽는다.

## macOS Native Signals

macOS에서는 Linux `/proc` 경로가 없으므로 `skid-monitor-agent`가 별도 MacBook signal sampler를
사용한다. 이 sampler는 `uptime`, `vm_stat`, `df`, `pmset`에서 load average, VM/memory 상태,
root filesystem 사용량, 배터리/AC 전원 상태를 수집한다.

MacBook signal은 OTLP metrics로 보내되 resource attribute `skid_monitor.source=macos`를 붙여
기존 Linux `system` live signal과 분리한다. 공통 OS 지표는 `system.*` metric 이름을 유지하고,
Mac 전용 전원/배터리/VM 지표는 `macos.*` prefix를 쓴다. 시리얼 번호, 하드웨어 UUID, 배터리 serial,
사용자 이름 같은 식별자는 수집하지 않는다.

## Windows Native Agent Target

Windows도 native agent 배포 대상이다. 다만 현재 mainline 구현은 Windows sampler와 `Source::Windows`
계약을 완료 상태로 두지 않는다. 목표 수집 경로는 PDH/Performance Counters, WMI/CIM, ETW, Event Log,
Service Control Manager 상태이며, 배포 산출물은 signed `.msi`와 Windows Service를 기준으로 둔다.
Windows runtime 검증은 Windows runner 또는 실제 Windows host에서 통과한 경우에만 완료로 기록한다.

## Agent Pipeline Config

`SKID_MONITOR_AGENT_CONFIG`에 JSON 설정 파일을 지정하면 agent를 Collector처럼
receiver/processor/exporter/pipeline 단위로 구성할 수 있다. 설정이 없으면 기존 env 기반 기본값
(`skid` exporter가 `SKID_MONITOR_CLIENT_ADDR`로 전송, device socket은 `127.0.0.1:9101`)을 쓴다.

```json
{
  "receivers": {
    "self_observation": { "enabled": true, "interval_secs": 15 },
    "device": { "enabled": true, "listen_addr": "127.0.0.1:9101" },
    "otlp": { "enabled": true, "grpc_addr": "127.0.0.1:4317" }
  },
  "processors": {
    "batch": { "type": "batch" }
  },
  "exporters": {
    "skid": { "type": "skid_client", "addr": "127.0.0.1:9000" },
    "debug": { "type": "logging", "include_json": false },
    "upstream": { "type": "otlp", "endpoint": "http://127.0.0.1:4317" }
  },
  "pipelines": {
    "metrics": { "receivers": ["self_observation", "device", "otlp"], "processors": ["batch"], "exporters": ["skid", "debug"] },
    "traces": { "receivers": ["self_observation", "device", "otlp"], "processors": ["batch"], "exporters": ["skid"] },
    "logs": { "receivers": ["self_observation", "device", "otlp"], "processors": ["batch"], "exporters": ["skid"] }
  }
}
```

## Observation Device Socket

관측기기/게이트웨이는 agent의 장비 소켓에 연결해 length-prefixed JSON `Signal` 프레임이나
임베디드 친화적인 `skid-edge-wire` compact metric frame을 보낼 수 있다.
기본 수신 주소는 `127.0.0.1:9101`이며,
`SKID_MONITOR_DEVICE_LISTEN_ADDR`로 바꿀 수 있다. 비활성화하려면
`SKID_MONITOR_DEVICE_LISTEN_ADDR=off`를 쓴다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 \
SKID_MONITOR_DEVICE_LISTEN_ADDR=127.0.0.1:9101 \
cargo run -p skid-monitor-agent
```

## Edge Physical Signals

STM32/ESP32 같은 MCU를 Kubernetes node로 취급하지 않는다. 대신 edge node 주변의 물리 계층과
환경 신호를 관측하는 작은 probe/proxy로 둔다.

예상 신호:

- 전원: 입력 전압, 배터리 상태, brownout, PoE 상태
- 열/환경: 온도, 습도, 팬 상태, enclosure 개폐
- 네트워크: 링크 업/다운, RSSI, 패킷 손실, LoRa/Wi-Fi/Ethernet 상태
- 장비 상태: watchdog reset, boot count, sensor fault, GPIO 상태

gateway나 agent에서 수신된 이 신호들은 OTLP metrics export request로 승격되며 resource attribute
`skid_monitor.source=edge_device`로 표시한다. 현재 구현이 붙이는 data point attribute는
`device_id`, `node_name`, `sensor`이며, `rack`, `zone` 같은 위치 attribute는 향후 식별
모델의 목표값으로 아직 코드에 없다.

현재 `skid-edge-agent`는 실제 센서 대신 mock sample을 agent 장비 소켓으로 전송한다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-edge-agent -- --once
```

## C# Client Extensions

`skid-monitor-client`는 선택적으로 .NET 확장 호스트를 sidecar 프로세스로 실행할 수 있다. Rust client는 TCP 수신과
콘솔 렌더링을 유지하고, 확장 호스트에는 newline-delimited JSON 이벤트를 stdin으로 전달한다.

```sh
dotnet build client/skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj
dotnet build client/skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/Skid.Monitor.Client.SampleExtension.csproj

SKID_MONITOR_DOTNET_EXTENSIONS=./client/skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/bin/Debug/netstandard2.1/Skid.Monitor.Client.SampleExtension.dll \
SKID_MONITOR_EXTENSION_HOST="dotnet run --project client/skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj" \
cargo run -p skid-monitor-client
```

Unity 6 managed plug-in 호환을 위해 확장 SDK와 sample extension은 `netstandard2.1`을 target으로
둔다. out-of-process extension host는 Unity plug-in이 아니라 sidecar 실행 파일이므로 `net8.0`을 유지한다.

확장 SDK, host, 런타임 sidecar 모델은 [client/skid-monitor-client/bindings/dotnet/README.md](client/skid-monitor-client/bindings/dotnet/README.md)와
[client RFC](client/skid-monitor-client/docs/rfcs/0002-csharp-extension-developer-experience.md)을 따른다.

## Planned Nodes

`skid-file-node`는 read-only file offer와 chunk download부터 시작한다. `sshfs`, `sftp`,
`rsync`, local directory backend는 구현 세부 driver로 두고, client에는 안전하게 받을 수 있는
파일 offer를 노출한다.

현재 진입점은 전송 구현 전 단계로 root별 파일 수/용량/가용성 metric만 장비 소켓으로 보낸다. 제품
범위에는 client가 offer를 보고 server 경유로 read-only chunk download를 요청하는 흐름이 포함된다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 \
cargo run -p skid-file-node -- --root logs=./logs --once
```

`skid-compute-advisor`는 병렬 처리가 가능한 기기의 CPU/GPU/RAM/VRAM/load/thermal/network
capability를 수집하고, 실제 원격 실행 전에 explainable route advice를 제공하는 방향으로 둔다.

현재 진입점은 원격 실행 없이 logical CPU 수와 placeholder route score만 장비 소켓으로 보낸다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-compute-advisor -- --once
```

## Quantum Telemetry

클라우드 양자 컴퓨팅 백엔드까지 관측 범위를 넓힐 수 있다. 다만 양자컴퓨터를 edge 장비처럼 직접
붙이는 것이 아니라, IBM Quantum, Amazon Braket, Azure Quantum 같은 서비스의 job/task API를
관측 소스로 연결한다.

예상 신호:

- QPU/backend 상태: online 여부, provider, backend name, qubit count
- 큐/작업: queued/running/completed/failed, queue depth, wait time, run time
- 실행 품질: shots, error mitigation 설정, result confidence, failure reason
- 비용/할당량: task count, quota 사용량, provider별 billing 단서

이 신호들은 `skid_protocol::metrics::Source::Quantum`으로 표시한다. 실제 quantum adapter는
각 provider SDK나 API를 호출하는 별도 crate로 두고, Skid Monitor 내부 계약에는 provider별 타입을
직접 노출하지 않는다.
