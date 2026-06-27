# skid-edge-agent Use Cases

## 1. Local Mock Edge Signal

agent device socket을 local로 열고 edge sample을 한 번 보낸다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-edge-agent -- --once
```

## 2. Gateway Physical Health Probe

factory gateway나 lab machine에서 enclosure temperature, input voltage, Wi-Fi RSSI 같은 주변 상태를
agent로 주기 전송한다. 현재 값은 mock이고 sensor backend는 future 작업이다.

## 3. Device Identity Smoke Test

`SKID_MONITOR_EDGE_DEVICE_ID`와 `SKID_MONITOR_EDGE_NODE`를 바꿔 client 화면에서 attribute가 기대대로
흐르는지 확인한다.

## 4. Sidecar Compatibility Check

same host 또는 same Pod 안에서 agent와 edge agent를 함께 띄워 device ingress framing과 source
attribute를 검증한다.
