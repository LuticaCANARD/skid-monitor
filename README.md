# Skid Monitor

Skid Monitor는 애플리케이션, 호스트, edge 장비, 파일 접근 노드, 분산 compute capability에서
나오는 신호를 하나의 가벼운 프로토콜로 모으는 실험적 모니터링/라우팅 툴킷이다.

이 repository는 SKID 계열 실험의 통합 지점이다. 다른 `skid-*` repository에서 확인한 요소는
먼저 RFC로 옮겨 설계를 고정하고, 실제 구현은 이 repository 안에서만 진행한다.

현재 워크스페이스는 여섯 개의 Rust crate와 client-side .NET binding 프로젝트로 나뉜다.

- `skid-protocol`: agent/client/edge adapter가 공유하는 OTLP 기반 직렬화 계약
- `skid-monitor-agent`: OpenTelemetry 및 Linux host/system 신호를 수집해 `Signal`로 전송하는 agent
- `skid-edge-agent`: edge 물리 계층/환경 신호를 수집해 `Signal`로 전송하는 agent
- `skid-file-node`: read-only file offer 후보 root를 관측 신호로 알리는 초기 file node
- `skid-compute-advisor`: 병렬 처리 capability와 route advice 후보를 알리는 초기 compute advisor
- `skid-monitor-client`: `Signal`을 받아 콘솔에 표시하고 C# 확장 호스트로 전달하는 client
- `skid-monitor-client/bindings/dotnet/`: 별도 SDK 라이브러리, out-of-process .NET extension host, sample extension

배포 경계와 Kubernetes 운용 판단은 [RFC 0001: Edge and Capability Node Deployment](docs/rfcs/0001-edge-capability-node-deployment.md)를 따른다.
클라이언트 UI와 C# extension 개발 방향은 [skid-monitor-client/docs](skid-monitor-client/docs/README.md)에 둔다.

다른 SKID 계열 repository에서 가져온 설계 후보는 RFC로 고정한다. `skid-node`의 설정/transport plane은
[RFC 0002](docs/rfcs/0002-node-config-and-transport-planes.md), device ingress frame은
[RFC 0003](docs/rfcs/0003-device-ingress-framing-and-safety.md), GPU/image workload 기반 compute
advisor는 [RFC 0004](docs/rfcs/0004-compute-workload-probes-and-route-advice.md), stream telemetry는
[RFC 0005](docs/rfcs/0005-stream-telemetry-and-media-preview.md)를 따른다.

## Server Metrics

`skid-monitor-agent`는 OpenTelemetry 자체 계측뿐 아니라 Linux 서버의 호스트 메트릭도 함께
수집한다. 현재 수집 범위는 CPU 사용률, load average, 메모리/스왑, uptime, 파일시스템 용량,
디스크 I/O, 네트워크 I/O, agent 프로세스 메모리/스레드/open FD 상태다.

모든 값은 `Signal::Metrics(ExportMetricsServiceRequest)`로 client에 전송된다. 서버 호스트
메트릭은 OTLP resource attribute `skid_monitor.source=system`으로 표시된다.

```sh
# terminal 1: 사람이 보는 client
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-client

# terminal 2: 서버 메트릭 수집 agent
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

예전 `MONITOR_CAT_*` 환경변수도 당분간 fallback으로 읽는다.

## Observation Device Socket

관측기기/게이트웨이는 agent의 장비 소켓에 연결해 같은 length-prefixed JSON `Signal` 프레임을
보낼 수 있다. 기본 수신 주소는 `127.0.0.1:9101`이며,
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

이 신호들은 OTLP metrics export request로 감싸며 resource attribute
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
dotnet build skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj
dotnet build skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/Skid.Monitor.Client.SampleExtension.csproj

SKID_MONITOR_DOTNET_EXTENSIONS=./skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/bin/Debug/net8.0/Skid.Monitor.Client.SampleExtension.dll \
SKID_MONITOR_EXTENSION_HOST="dotnet run --project skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj" \
cargo run -p skid-monitor-client
```

확장 SDK, host, 런타임 sidecar 모델은 [skid-monitor-client/bindings/dotnet/README.md](skid-monitor-client/bindings/dotnet/README.md)와
[client RFC](skid-monitor-client/docs/rfcs/0001-csharp-extension-developer-experience.md)을 따른다.

## Planned Nodes

`skid-file-node`는 read-only file offer와 chunk download부터 시작한다. `sshfs`, `sftp`,
`rsync`, local directory backend는 구현 세부 driver로 두고, client에는 안전하게 받을 수 있는
파일 offer를 노출한다.

현재 진입점은 실제 전송을 열지 않고 root별 파일 수/용량/가용성 metric만 장비 소켓으로 보낸다.

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
