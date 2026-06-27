# Monitor-cat

Monitor-cat은 애플리케이션, Kubernetes, 호스트, 엣지 물리 환경에서 발생하는 신호를
하나의 가벼운 관측 프로토콜로 모으는 실험적 모니터링 도구다.

현재 워크스페이스는 네 부분으로 나뉜다.

- `interface`: server/client/edge adapter가 공유하는 OTLP 기반 직렬화 계약
- `moniter-server`: OpenTelemetry 기반 신호를 수집해 `Signal`로 전송하는 agent
- `moniter-edge-agent`: 엣지 물리 계층/환경 신호를 수집해 `Signal`로 전송하는 agent
- `moniter-client`: `Signal`을 받아 사람이 볼 수 있게 표시하는 client

## Server metrics

`moniter-server`는 OpenTelemetry 자체 계측뿐 아니라 Linux 서버의 호스트 메트릭도 함께 수집한다.
현재 수집 범위는 CPU 사용률, load average, 메모리/스왑, uptime, 파일시스템 용량,
디스크 I/O, 네트워크 I/O, monitor-cat 서버 프로세스 메모리/스레드/open FD 상태다.

모든 값은 `Signal::Metrics(ExportMetricsServiceRequest)`로 client에 전송된다. 서버 호스트
메트릭은 OTLP resource attribute `monitor_cat.source=system`으로 표시된다.

```sh
# terminal 1: 사람이 보는 client
MONITOR_CAT_CLIENT_ADDR=127.0.0.1:9000 cargo run -p moniter-client

# terminal 2: 서버 메트릭 수집 agent
MONITOR_CAT_CLIENT_ADDR=127.0.0.1:9000 cargo run -p moniter-server
```

## Observation device socket

관측기기/게이트웨이는 server의 장비 소켓에 연결해 같은 length-prefixed JSON `Signal` 프레임을 보낼 수 있다.
기본 수신 주소는 `127.0.0.1:9101`이며, `MONITOR_CAT_DEVICE_LISTEN_ADDR`로 바꿀 수 있다.
비활성화하려면 `MONITOR_CAT_DEVICE_LISTEN_ADDR=off`를 쓴다.

```sh
MONITOR_CAT_CLIENT_ADDR=127.0.0.1:9000 \
MONITOR_CAT_DEVICE_LISTEN_ADDR=127.0.0.1:9101 \
cargo run -p moniter-server
```

## Edge physical signals

STM32/ESP32 같은 MCU를 Kubernetes node로 취급하지 않는다. 대신 edge node 주변의
물리 계층과 환경 신호를 관측하는 작은 probe/proxy로 둔다.

예상 신호:

- 전원: 입력 전압, 배터리 상태, brownout, PoE 상태
- 열/환경: 온도, 습도, 팬 상태, enclosure 개폐
- 네트워크: 링크 업/다운, RSSI, 패킷 손실, LoRa/Wi-Fi/Ethernet 상태
- 장비 상태: watchdog reset, boot count, sensor fault, GPIO 상태

이 신호들은 OTLP metrics export request로 감싸며 resource attribute
`monitor_cat.source=edge_device`로 표시한다. 현재 구현이 붙이는 data point attribute는
`device_id`, `node_name`, `sensor`이며, `rack`, `zone` 같은 위치 attribute는 향후 식별
모델의 목표값으로 아직 코드에 없다. 초기 구현은 기존 length-prefixed JSON over TCP를
재사용한다. CBOR/postcard 같은 compact encoding은 실제로 펌웨어에 직접 올리는 probe가
생기거나 대역폭 제약이 측정될 때 추가한다(현재 edge agent는 Linux 위 Rust 바이너리다).

현재 `moniter-edge-agent`는 실제 센서 대신 mock sample을 server 장비 소켓으로 전송한다.

```sh
MONITOR_CAT_DEVICE_ADDR=127.0.0.1:9101 cargo run -p moniter-edge-agent -- --once
```

`moniter-edge-agent`는 `moniter-server`와 별도로 배포되는 현장 probe이며, server의 장비
소켓으로 edge 신호를 push한다. 운영 배포는 단일 바이너리와 systemd 서비스를 기본값으로
두며, 자세한 관계와 설치 전략은 [edge agent deployment](docs/edge-agent-deployment.md)를
따른다.

## Quantum telemetry

"모든 것을 볼 수 있는 것"의 범위를 클라우드 양자 컴퓨팅 백엔드까지 넓힐 수 있다.
다만 양자컴퓨터를 edge 장비처럼 직접 붙이는 것이 아니라, IBM Quantum, Amazon Braket,
Azure Quantum 같은 서비스의 job/task API를 관측 소스로 연결한다.

예상 신호:

- QPU/backend 상태: online 여부, provider, backend name, qubit count
- 큐/작업: queued/running/completed/failed, queue depth, wait time, run time
- 실행 품질: shots, error mitigation 설정, result confidence, failure reason
- 비용/할당량: task count, quota 사용량, provider별 billing 단서

이 신호들은 `interface::metrics::Source::Quantum`으로 표시한다. 실제 quantum adapter는
각 provider SDK나 API를 호출하는 별도 크레이트로 두고, monitor-cat 내부 계약에는
provider별 타입을 직접 노출하지 않는다.
