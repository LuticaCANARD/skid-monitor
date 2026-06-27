# RFC 0002: Node Configuration and Transport Planes

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `docs/rfcs/0002-node-config-and-transport-planes.md` |
| Scope | `skid-monitor-agent`, `skid-edge-agent`, `skid-file-node`, `skid-compute-advisor`, future stream nodes |
| Related | RFC 0001, RFC 0003, `LuticaCANARD/skid-node` |
| Decision Type | Configuration, deployment contract, transport boundary |

## Abstract

이 RFC는 `LuticaCANARD/skid-node`의 `NetworkNodeArgs` 설계에서 가져올 요소를
Skid Monitor에 맞게 줄여 정의한다. 핵심은 모든 node가 환경변수만으로 흩어져 설정되는 상태를
벗어나, `node`, `transport`, `interface`, `binding`, `tunnel`을 한 장의 설정 파일에서 설명하게
하는 것이다.

현재 코드는 이 RFC를 아직 구현하지 않는다. 기존 환경변수 경로는 계속 동작해야 하며, 향후 설정
파일이 들어오면 설정 파일을 기본값으로 삼고 환경변수를 마지막 override로 적용한다.

## Imported Ideas

`skid-node`에서 그대로 가져올 원칙은 다음이다.

- `listen`과 `connect`를 명시적으로 나눈다.
- `control_transport`와 `data_transport`를 분리한다.
- resource/stream은 `interface`와 `binding`으로 선언한다.
- `start`, `relay`, `end` 같은 역할은 network topology를 설명하는 값이지 binary 이름이 아니다.
- 설정은 JSON/YAML을 모두 허용하지만, 사람이 쓰는 기본 포맷은 YAML이다.
- 이름 참조는 load 단계에서 검증한다. 잘못된 참조는 agent를 부분 기동하지 않고 실패시킨다.

Skid Monitor에서는 이를 더 좁게 적용한다. 이 프로젝트의 첫 목표는 remote resource 실행이 아니라
관측 신호를 안정적으로 모으는 것이다. 따라서 `runner`와 `binding`은 처음에는 "무엇을 관측할 수
있는가"를 설명하고, 직접 실행은 별도 RFC까지 열지 않는다.

## Decision Summary

- `skid-monitor.yaml`을 표준 설정 파일 이름으로 둔다.
- 기본 탐색 순서는 `--config`, `SKID_MONITOR_CONFIG`, `./skid-monitor.yaml`이다.
- 환경변수는 유지하되, 설정 파일보다 높은 우선순위의 override로만 해석한다.
- node 종류는 `collector`, `edge_device`, `file_node`, `compute_advisor`, `stream_node`를 1차
  값으로 둔다.
- transport는 `tcp`, `udp`, `quic`, `rtp_udp`, `custom` label을 허용한다. 현재 구현은 `tcp`만
  실행하고 나머지는 검증 가능한 선언으로 둔다.
- binding은 `kind = metric | resource | stream | workload` 중 하나로 둔다.
- 실제 파일 전송, remote execution, media forwarding은 이 RFC의 범위 밖이다.

## Configuration Shape

초기 YAML 예시는 다음과 같다.

```yaml
node_name: lab-gateway-1
roles:
  - collector

client:
  connect: 127.0.0.1:9000
  protocol: skid_signal_json

device_ingress:
  listen: 127.0.0.1:9101
  protocol: skid_device_v1
  interface: loopback
  max_frame_bytes: 16777216
  max_connections: 128

transports:
  - name: device-control
    protocol: tcp
    listen: 127.0.0.1:9101
    plane: control
  - name: stream-metadata
    protocol: tcp
    connect: 127.0.0.1:9120
    plane: control
  - name: media-data
    protocol: rtp_udp
    connect: 239.0.0.1:5004
    plane: data

interfaces:
  - name: edge-env
    kind: builtin
    adapter: skid-edge-agent
    emits:
      - edge.temperature
      - edge.voltage.input
  - name: file-offers
    kind: builtin
    adapter: skid-file-node
    args:
      roots:
        - label: logs
          path: /var/log/my-service
  - name: gpu-probe
    kind: builtin
    adapter: skid-compute-advisor
    args:
      workload_probes:
        enabled: false
  - name: camera-stream
    kind: external
    adapter: skid-stream-node
    args:
      status_url: http://127.0.0.1:8080/status

bindings:
  - name: local-edge-metrics
    kind: metric
    interface: edge-env
    control_transport: device-control
  - name: log-file-offer
    kind: resource
    resource_type: file_roots
    interface: file-offers
    capabilities:
      - list
      - read_offer
    control_transport: device-control
  - name: gpu-route-advice
    kind: workload
    resource_type: gpu
    interface: gpu-probe
    capabilities:
      - observe
      - score
    control_transport: device-control
  - name: camera-preview
    kind: stream
    resource_type: webrtc
    interface: camera-stream
    capabilities:
      - observe
      - preview_metadata
    control_transport: stream-metadata
    data_transport: media-data
```

## Validation Rules

설정 loader는 다음을 검증한다.

- `node_name`은 비어 있으면 안 된다.
- `roles`는 하나 이상이어야 한다.
- `device_ingress.listen=0.0.0.0:*`는 `security.allow_public_ingress=true`가 없으면 실패한다.
- `transports[].name`, `interfaces[].name`, `bindings[].name`은 각 범위에서 유일해야 한다.
- `transports[].protocol`은 비어 있으면 안 된다.
- `bindings[].interface`는 존재하는 interface를 참조해야 한다.
- `bindings[].control_transport`와 `bindings[].data_transport`는 존재하는 transport를 참조해야 한다.
- `binding.kind=stream`인데 `data_transport`가 없으면 warning으로 시작하고, media 자체는 전달하지
  않는 metadata-only stream으로 취급한다.
- `interface.kind=external`은 처음에는 command를 실행하지 않고, 선언과 metric label에만 사용한다.

## Environment Overrides

기존 환경변수는 당장 제거하지 않는다.

| 환경변수 | 설정 파일 필드 |
| --- | --- |
| `SKID_MONITOR_CLIENT_ADDR` | `client.connect` |
| `SKID_MONITOR_DEVICE_LISTEN_ADDR` | `device_ingress.listen` |
| `SKID_MONITOR_DEVICE_ADDR` | node sender의 `device_ingress.connect` |
| `SKID_FILE_NODE_NAME` | file node `node_name` |
| `SKID_COMPUTE_ADVISOR_NODE` | compute advisor `node_name` |

우선순위는 CLI flag, 환경변수, 설정 파일, 코드 기본값 순서다.

## Implementation Plan

1. `skid-protocol`이 아닌 각 binary에 `config` 모듈을 둔다. 프로토콜 crate는 wire contract만 가진다.
2. `serde_yaml`을 workspace dependency로 추가한다.
3. `skid-monitor-agent`부터 `--config`와 `SKID_MONITOR_CONFIG`를 읽는다.
4. current env var path를 config override 계층으로 옮긴다.
5. `skid-file-node`, `skid-compute-advisor`, future `skid-stream-node`에 같은 loader 패턴을 적용한다.
6. 설정 검증 실패는 `stderr`에 field path를 출력하고 non-zero exit한다.
7. RFC 0003의 device frame 설정값을 `device_ingress.protocol`에 연결한다.

## Non-Goals

- `skid-node`의 전체 mesh runtime을 vendoring하지 않는다.
- shell/executable runner를 통해 remote command를 실행하지 않는다.
- Kubernetes scheduler나 service mesh 기능을 대체하지 않는다.
- media bytes, file chunks, compute inputs를 device socket에 싣지 않는다.

## Open Questions

- 설정 파일을 모든 binary가 독립적으로 읽을지, agent가 node 설정을 배포할지 결정해야 한다.
- node enrollment가 들어오면 `node_name`과 credential의 binding 위치를 다시 정해야 한다.
- `custom` transport dispatch는 별도 trait을 둘지, sidecar Unix socket만 받을지 정해야 한다.
