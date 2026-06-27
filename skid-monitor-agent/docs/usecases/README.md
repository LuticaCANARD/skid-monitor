# skid-monitor-agent Use Cases

## 1. Local Host Metric Collector

`skid-monitor-client`를 `127.0.0.1:9000`에 띄운 뒤 agent를 실행하면 agent가 자체 telemetry와 Linux
system metrics를 client로 보낸다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

## 2. Device Ingress Gateway

agent는 기본적으로 `127.0.0.1:9101`에서 edge/file/compute node의 push를 받는다. 같은 host나 same-Pod
sidecar에서 capability node를 붙이는 개발 형태에 적합하다.

## 3. Site Gateway

trusted LAN 또는 overlay IP에 `SKID_MONITOR_DEVICE_LISTEN_ADDR`를 bind하면 여러 capability node가
하나의 site agent로 metric을 보낼 수 있다. public `0.0.0.0` bind는 인증과 제한이 들어가기 전까지
use case가 아니다.

## 4. Client Forwarder

device socket으로 받은 `Signal`과 agent 자체 수집 신호를 같은 client transport로 보낸다. 현재는
신호 1개당 connect, write, close를 수행한다.
