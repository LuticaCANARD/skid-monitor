# skid-compute-advisor Use Cases

## 1. CPU Capability Smoke Test

agent device socket에 현재 host의 logical CPU 수를 보낸다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-compute-advisor -- --once
```

## 2. Executor Disabled Declaration

client나 extension은 `executor_enabled=false` attribute를 보고 이 node가 실행 권한을 제공하지 않는
관측 node임을 알 수 있다.

## 3. Route Advice Seed

현재 placeholder score를 통해 client rendering과 downstream extension이 route advice 형태를 먼저
실험할 수 있다. 실제 scheduling decision에는 쓰지 않는다.

## 4. Future Probe Harness

memory, GPU, thermal, external workload probe가 추가될 때 같은 device ingress와 `compute_advisor.*`
namespace를 재사용한다.
