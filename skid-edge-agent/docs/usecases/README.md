# skid-edge-agent Product Use Cases

`skid-edge-agent`는 제품에서 현장 장비 주변의 물리 신호를 Skid Monitor로 끌어오는 probe다. 현재는
mock sample이지만, use case는 실제 gateway/장비 환경을 기준으로 정의한다.

## Kubernetes 운용 호환성

edge agent는 Kubernetes에서도 lab/mock DaemonSet으로는 운용할 수 있지만, 실제 GPIO/I2C/serial 장비를
읽는 순간 host device 권한이 필요하다. production 기본값은 bare-metal/systemd gateway이고, k8s에서는
권한이 명시된 hardware gateway node에 한정한다.

개발 step:

1. MVP: mock edge metric은 unprivileged Pod나 same-Pod sidecar로 실행 가능하게 유지한다.
2. 다음 단계: hardware backend는 node selector, toleration, explicit device mount, read-only root
   filesystem 예시를 따로 둔다.
3. Production: privileged Pod를 기본값으로 삼지 않고, 필요한 device만 허용하며 device identity와
   Kubernetes node identity를 함께 attribute로 보낸다.

## Use Case 1: 현장 Gateway 건강 상태를 본다

제품 경험: 공장 라인, lab rack, edge gateway의 온도, 입력 전압, Wi-Fi RSSI, watchdog reset을 monitor
client에서 본다. 운영자는 서버 metric만으로 알 수 없는 물리 문제를 조기에 확인한다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-edge-agent -- --once
```

개발 step:

1. MVP: deterministic mock metric으로 end-to-end signal path를 검증한다.
2. 다음 단계: Linux gateway에서 읽을 수 있는 thermal, network interface, uptime backend를 붙인다.
3. Production: GPIO/I2C/serial backend를 adapter로 분리하고 sensor failure metric을 추가한다.

## Use Case 2: 장비 Identity별 상태를 구분한다

제품 경험: 같은 site에 edge node가 여러 개 있어도 `device_id`와 `node_name`으로 각 장비의 상태를
분리해서 본다.

개발 step:

1. MVP: `SKID_MONITOR_EDGE_DEVICE_ID`, `SKID_MONITOR_EDGE_NODE` env var를 attribute로 보낸다.
2. 다음 단계: config file에서 location, rack, zone 같은 label을 받는다.
3. Production: enrollment credential과 `device_id`를 묶어 spoofing을 줄인다.

## Use Case 3: Brownout / 환경 이상 징후를 제품 이벤트로 만든다

제품 경험: 입력 전압 하락, 온도 상승, watchdog reset 증가가 client와 extension에서 alert 후보로
보인다.

개발 step:

1. MVP: `edge.voltage.input`, `edge.temperature`, `edge.watchdog.resets`를 안정적으로 보낸다.
2. 다음 단계: threshold classification을 client extension에서 실험한다.
3. Production: alert policy와 hysteresis를 별도 정책 계층으로 분리한다.

## Use Case 4: 설치 전 Device Ingress 호환성을 검증한다

제품 경험: 현장 설치 전에 edge agent만 `--once`로 실행해 agent device ingress와 client render가
정상인지 확인한다.

개발 step:

1. MVP: `--once` smoke test를 유지한다.
2. 다음 단계: send failure exit code와 structured stderr를 추가한다.
3. Production: installer가 post-install check로 이 smoke test를 실행한다.

## Use Case 5: 현장 Camera / Snapshot Provider 상태를 본다

제품 경험: edge gateway에 연결된 카메라나 snapshot provider의 up/down, 해상도, fps, 최신 snapshot age,
preview endpoint 존재 여부를 monitor client에서 본다. 실제 이미지는 사용자가 명시적으로 preview를 켠
extension/GUI가 provider data plane에서 가져오고, edge agent는 raw image bytes를 device ingress로 보내지 않는다.

개발 step:

1. MVP: mock media provider mode로 `stream.provider.*`, `stream.endpoint.*`, `stream.snapshot.*` metric을 보낸다.
2. 다음 단계: `status_http`나 `snapshot_http` adapter로 provider health와 snapshot freshness를 관측한다.
3. Production: endpoint redaction, preview permission, audit id를 extension/GUI boundary와 연결한다.
