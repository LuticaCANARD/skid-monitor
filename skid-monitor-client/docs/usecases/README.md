# skid-monitor-client Use Cases

## 1. Console Monitor

agent가 보내는 metrics, traces, logs를 terminal에서 바로 확인한다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-client
```

## 2. Signal Debug Sink

edge/file/compute node와 agent의 wire contract를 확인할 때 client를 단순 수신 sink로 사용한다. 각
신호는 count와 주요 data point를 text로 출력한다.

## 3. C# Extension Host

Rust client는 기본 수신과 렌더링을 유지하고, signal event를 .NET extension host stdin으로 넘긴다.
extension은 classification, avatar state 변환, Unity bridge 같은 후처리를 담당할 수 있다.

## 4. Future Preview Coordinator

stream telemetry가 들어오면 core client는 endpoint 존재와 stream 상태를 요약하고, 실제 preview는
extension 또는 future GUI layer가 명시적 사용자 동의 후 연다.
