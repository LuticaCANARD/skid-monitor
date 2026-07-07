# Client RFC 0003: Agent Pipeline Visibility

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-07-02 |
| File | `skid-monitor-client/docs/rfcs/0003-agent-pipeline-visibility.md` |
| Scope | `skid-monitor-client`, `skid-monitor-fe`, `skid-monitor-agent` integration |
| Protocol | length-prefixed JSON `Signal` over TCP |
| Decision Type | Client observation contract for agent exporter pipelines |

## Abstract

`skid-monitor-agent`가 Collector-like `receiver/processor/exporter/pipeline` 설정을 갖더라도,
client는 agent 내부 설정을 직접 해석하지 않는다. client는 `SKID_MONITOR_CLIENT_ADDR` 또는
client 전용 `SKID_MONITOR_CLIENT_ADDRS` listen endpoint에서 length-prefixed JSON `Signal`을
수신하고, agent의 `skid_client` exporter가 대응 주소로 보낸 metrics/traces/logs를 표시한다.

즉 client의 관점에서 exporter 설정 체계는 "보낼지 말지, 어디로 보낼지"를 agent가 결정하는 upstream
routing policy다. client는 Signal consumer이며, pipeline 설정의 source of truth는 agent config다.

## Decision Summary

- client가 보려면 먼저 `SKID_MONITOR_CLIENT_ADDR` 또는 `SKID_MONITOR_CLIENT_ADDRS`에서 TCP listener를 열어야 한다.
- agent config의 `skid_client` exporter는 client listener 중 하나와 같은 주소를 가리켜야 한다.
- 다중 노드 agent는 FE/client가 여러 listener를 열고, 각 agent/exporter가 자기 노드에 대응하는 단일 주소로 보낸다.
- native FE는 한 창의 `Nodes` table에서 명시 node attribute 또는 listener endpoint fallback 기준으로 노드 행을 분리한다.
- metrics/traces/logs pipeline 각각에 `skid_client` exporter 이름이 포함되어야 client에 도착한다.
- client는 `Signal::{Metrics, Traces, Logs}` payload를 그대로 수신한다.
- OTLP receiver, logging exporter, upstream OTLP exporter 존재 여부는 client protocol을 바꾸지 않는다.
- 현재 실행 가능한 native viewer는 `skid-monitor-fe`이며, `skid-monitor-client` crate는 receiver와
  extension host 연동을 제공하는 library surface다.

## Runtime Shape

```text
instrumented app / device / self-observation
        |
        v
skid-monitor-agent
  receivers: self_observation, device, otlp
  processors: batch, ...
  exporters: skid_client, logging, otlp
        |
        | skid_client exporter
        | length-prefixed JSON Signal
        v
skid-monitor-client::receiver
        |
        v
skid-monitor-client::receiver_loop
        |
        +-- skid-monitor-fe dashboard
        +-- optional extension host
        +-- future skid-monitor-tui
```

The client-side invariant is small: if a `Signal` reaches the TCP receiver, client surfaces render or forward it.
The client does not need to know which agent receiver produced the signal or which processors ran before export.

## Operator Workflow

Start the current native viewer first so it owns the listen socket.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-fe
```

Then start the agent with a pipeline config that exports to the same address.

```sh
SKID_MONITOR_AGENT_CONFIG=skid-monitor-agent/examples/agent-config.json cargo run -p skid-monitor-agent
```

For multiple node agents, start the viewer with a comma-separated listener list
and point each node agent at one endpoint.

```sh
SKID_MONITOR_CLIENT_ADDRS=127.0.0.1:9000,127.0.0.1:9001 cargo run -p skid-monitor-fe

SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9001 cargo run -p skid-monitor-agent
```

The minimal client-facing exporter shape is:

```json
{
  "exporters": {
    "skid": {
      "type": "skid_client",
      "addr": "127.0.0.1:9000"
    }
  },
  "pipelines": {
    "metrics": { "exporters": ["skid"] },
    "traces": { "exporters": ["skid"] },
    "logs": { "exporters": ["skid"] }
  }
}
```

If a pipeline omits `skid`, that signal type is intentionally invisible to the client. For example, metrics can be
sent to `skid` and `logging`, while traces are sent only to an upstream OTLP backend.

## OTLP Receiver Visibility

To show telemetry from an OpenTelemetry-instrumented app in the client, the app sends OTLP to the agent, and the
agent forwards accepted signals through `skid_client`.

Agent receiver config:

```json
{
  "receivers": {
    "otlp": {
      "enabled": true,
      "grpc_addr": "127.0.0.1:4317"
    }
  }
}
```

Application-side environment example:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317
```

The client still listens only on the configured Skid client signal endpoint(s). It does not expose an OTLP receiver
and does not accept OTLP directly.

## Client Responsibilities

- Bind the configured client address or address list before the agent starts exporting.
- Decode the length-prefixed JSON `Signal` frame.
- Render metrics, traces, and logs without assuming a single receiver source.
- Surface receive errors clearly, especially bind failures and malformed frames.
- Preserve extension delivery semantics: received signals may still be forwarded to the .NET extension host.

## Agent Responsibilities

- Own the exporter/pipeline config and validate references.
- Send client-visible signals through `skid_client`.
- Treat `logging` and `otlp` exporters as separate fan-out destinations, not as client protocol changes.
- Keep the `Signal` wire shape stable unless a separate protocol RFC changes it.

## Failure Modes

- If client is not running, `skid_client` export fails with connection refused.
- If client and agent use different addresses, no signals appear in the dashboard.
- If a pipeline omits `skid`, that signal type is not shown by design.
- If another process owns `SKID_MONITOR_CLIENT_ADDR`, the client fails to bind and should report the bind error.
- If one address in `SKID_MONITOR_CLIENT_ADDRS` fails to bind, the client reports that listener error and continues
  with the listeners that did bind.
- If OTLP receiver is disabled, OpenTelemetry apps sending to `127.0.0.1:4317` will not appear in the client.

## Non-Goals

- client does not edit or hot-reload agent pipeline config.
- client does not become an OTLP receiver.
- client does not implement OpenTelemetry Collector processors.
- client does not infer dropped signal types unless agent later emits pipeline health/status signals.

## Open Questions

- Should the agent emit pipeline health metrics so the client can show exporter failure counts?
- Should the client display the agent receiver source (`self_observation`, `device`, `otlp`) when available?
- Should `skid-monitor-client` grow a small binary again, or should `skid-monitor-fe` remain the primary viewer?
- Should the client expose a config mismatch hint when it sees no signals for a long interval?

## MVP Scope

1. Use `skid-monitor-fe` as the primary visual client.
2. Keep `skid-monitor-client::receiver` as the shared TCP bind/read layer.
3. Keep `skid-monitor-client::receiver_loop` as the shared app-facing receive loop for GUI/TUI clients.
4. Document that client visibility requires `skid_client` in the relevant agent pipeline.
5. Add future agent pipeline health signals in a separate RFC if operator feedback becomes necessary.
