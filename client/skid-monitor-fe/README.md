# Skid Monitor FE

Native and WebAssembly egui control-room frontend for Skid Monitor.

It listens on the same TCP signal endpoint as `skid-monitor-client` and renders
incoming OTLP metrics, traces, and logs as an operator-focused dashboard.

The layout adapts between a two-column control-room view and a stacked narrow
view. Numeric metrics keep a short in-memory history and render lightweight
sparkline trends without an extra plotting dependency.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-fe
```

The frontend defaults to the `low-spec` feature, which keeps the existing glow
renderer and Linux software GL fallback. For the higher-spec wgpu renderer:

```sh
cargo run -p skid-monitor-fe --features high-spec
```

For a pure wgpu build without the default glow backend:

```sh
cargo run -p skid-monitor-fe --no-default-features --features high-spec
```

Linux에서는 Mesa/Zink/Vulkan driver 상태에 따라 `failed to choose pdev` 같은 렌더러 초기화 오류가
날 수 있다. 이 frontend는 control-room UI라 기본값으로 software Mesa GL(`llvmpipe`)을 사용한다.
또한 Wayland compositor 연결이 끊기며 `Broken pipe` / `WinitEventLoop(ExitFailure(1))`가 나는
환경을 피하기 위해, `DISPLAY`가 있으면 기본값으로 X11/XWayland backend를 사용한다.

GPU 경로를 강제로 쓰고 싶을 때만 다음처럼 실행한다.

```sh
SKID_MONITOR_FE_USE_GPU=1 cargo run -p skid-monitor-fe
```

Wayland backend를 강제로 쓰고 싶을 때만 다음처럼 실행한다.

```sh
SKID_MONITOR_FE_USE_WAYLAND=1 cargo run -p skid-monitor-fe
```

Start an agent in another terminal with the same address:

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

## Browser frontend

The browser build reuses the same pages, components, models, and dashboard
state as the native frontend. Platform adapters provide WebGPU rendering,
WebSocket ingress, and browser `localStorage` persistence while the native
build keeps its TCP listener and SQLite database.

Install the WASM target and Trunk, then serve the frontend from this directory:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
cd client/skid-monitor-fe
trunk serve
```

Open <http://127.0.0.1:8080>. Add a `ws://` or `wss://` endpoint from the
Ingress connections control, or preconfigure one with the query string:

```text
http://127.0.0.1:8080/?ingress=ws://127.0.0.1:9100/signals
```

Browsers cannot bind the agent's raw TCP listener. The WebSocket endpoint must
bridge the existing signal transport and send either a JSON-serialized
`skid_protocol::Signal` text message or the same JSON bytes as an ArrayBuffer.
The native application continues to accept the existing length-prefixed TCP
frames without any protocol change.

For multiple node agents, open one frontend listener per node-facing endpoint.
`SKID_MONITOR_CLIENT_ADDRS` is only read by the frontend/client side; each agent
still exports to one `SKID_MONITOR_CLIENT_ADDR` or one `skid_client.addr`.
The `Nodes` table keeps the endpoints in one window and shows node, endpoint,
source, service, counters, latest value, and last-seen age.

```sh
SKID_MONITOR_CLIENT_ADDRS=127.0.0.1:9000,127.0.0.1:9001 cargo run -p skid-monitor-fe

# node-a
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent

# node-b
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9001 cargo run -p skid-monitor-agent
```
