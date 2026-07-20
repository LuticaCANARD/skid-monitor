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
renderer and Linux software GL fallback. The Character panel and its 2D
reactions are available in both `low-spec` and `high-spec`; `high-spec` adds the
native WGPU VRM viewport described below. For the higher-spec renderer:

```sh
cargo run -p skid-monitor-fe --features high-spec
```

For a pure wgpu build without the default glow backend:

```sh
cargo run -p skid-monitor-fe --no-default-features --features high-spec
cargo test -p skid-monitor-fe --lib --no-default-features --features high-spec
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

## Configurable character and VRM

Open **Settings > Character reactions** to configure the character shown for a
selected server node.

- Set a character name and, in the native client, an optional `.png`, `.jpg`,
  `.jpeg`, or `.vrm` model path. You can also drop a `.vrm` file onto the native
  window and then apply the draft. An empty path uses the built-in character.
- Configure the `idle`, `warning`, and `critical` actions independently.
- Each action selects one bounded UI motion: `Still`, `Pulse`, `Bounce`, or
  `Shake`, plus an optional custom speech-bubble message.
- The native frontend stores the reaction profile in its SQLite state database.
  Apply stays pending until SQLite acknowledges the profile write; profile and
  shutdown commands are prioritized over queued telemetry state writes.
  The browser frontend stores it in the same tenant-scoped `localStorage`
  boundary as browser signal state (or the legacy local scope) and retains the
  built-in model because native filesystem model paths cannot be loaded in a
  browser build.

The state source remains the deterministic built-in alert engine. CPU, memory,
file-root, receiver, and extension alert rules keep their fixed thresholds and
severities; this UI configures how selected-node metric and listener receiver
states are presented, not how alerts are evaluated. Frontend extension-host
errors remain visible in the alert/event UI but are not treated as a selected
server's Character state. Repeated samples for an already-firing alert do not
create a new reaction transition.

The native `high-spec` build validates VRM 1.0 and legacy VRM 0.x GLB files and
renders embedded meshes, node transforms, rest-pose skinning, and base-color
textures in a depth-enabled WGPU viewport:

```sh
cargo run -p skid-monitor-fe --no-default-features --features high-spec
```

The default `low-spec` build does not include the VRM/glTF renderer and uses the
built-in fallback for `.vrm` paths. The current VRM renderer is a static rest-pose
preview: `Still`/`Pulse`/`Bounce`/`Shake` transform the bounded viewport rather
than running skeletal clips. MToon-specific shading, expressions, SpringBone,
look-at/constraints, glTF animation, VRMA, and arbitrary scripts are not run.
VRM 1.0 requires valid `name`, non-empty `authors`, `licenseUrl`, and required
humanoid bone bindings; the legacy 0.0 extension uses its own required bone set.
External buffer/image URIs and unsupported required glTF extensions are rejected. See
[`docs/otel-vrm-dance-status.md`](docs/otel-vrm-dance-status.md) for the exact
boundary and future animation/Unity paths.

Native image/VRM decoding is size-bounded and serialized on one background
loader. Image textures are installed on the UI thread, while VRM texture and
mesh resources are created in the WGPU render callback. Failed or stale loads
keep the built-in character available.

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

Open <http://127.0.0.1:8080>. Browser ingress has two explicit modes:

- **Cloud client API (`https://`, or numeric loopback `http://`)** uses OIDC authentication,
  one-time stream tickets, durable cursor replay, and automatic reconnect.
- **Raw bridge (`ws://` or `wss://`)** preserves the original WebSocket bridge
  behavior. It accepts raw `Signal` JSON/binary frames and `SignalRecord` JSON,
  but deliberately ignores record cursors and provides no replay guarantee.

Enter either kind of endpoint in the Ingress connections control. A raw bridge
can still be preconfigured with the existing query string:

```text
http://127.0.0.1:8080/?ingress=ws://127.0.0.1:9100/signals
```

The current dashboard model holds one security scope at a time. Cloud mode
therefore permits exactly one client API endpoint and cannot be mixed with raw
bridge connections. Raw mode continues to support multiple bridge endpoints.

Browsers cannot bind the agent's raw TCP listener. The WebSocket endpoint must
bridge the existing signal transport and send either a JSON-serialized
`skid_protocol::Signal` text message or the same JSON bytes as an ArrayBuffer.
The native application continues to accept the existing length-prefixed TCP
frames without any protocol change.

### OIDC cloud mode

Serve the frontend and Rust client API behind the **same origin/reverse proxy**.
The adapter enforces this before reading the session token, preventing a crafted
`client_api` parameter from forwarding a bearer token to another origin. Route
the split client-access server under an origin-local path; cross-origin cloud
API endpoints are intentionally rejected rather than delegated to CORS.

The hosting OIDC Authorization Code + PKCE shell is responsible for login and
token refresh. Immediately before starting/reconnecting the Rust frontend, it
must place the current OIDC access token in browser `sessionStorage` under this
exact key:

```text
skid-monitor.oidc.access_token
```

Do not put the access token in a URL, `localStorage`, frontend configuration,
console output, or application logs. The Rust adapter reads the token afresh
from `sessionStorage` for each ticket request and does not retain it in
application state. The OIDC shell should remove the key on logout.
During migration the adapter also reads the legacy
`skid-monitor.keycloak.access_token` key when the new key is absent. A new key
that exists but is empty or invalid fails closed instead of falling back; the
shell should remove both keys on logout.

Start cloud mode by entering an absolute `https://` client API base URL, or by
supplying `client_api` at startup (URL-encode the parameter in production):

```text
https://monitor.example/?client_api=https%3A%2F%2Fmonitor.example
```

Plain `http://` is accepted only for numeric loopback hosts such as
`127.0.0.1`; hostname and non-loopback HTTP endpoints are rejected before the
adapter reads or sends the access token.

For each connection attempt the adapter performs a cache-disabled
`POST /v1/stream-tickets` with `Authorization: Bearer ...`, then consumes the
ticket exactly once through
`wss://.../v1/stream?ticket=...&after=<cursor>`. Tickets and tokens are never
included in application messages or logs. On close it rereads the latest
session token, requests a fresh ticket, and retries with exponential backoff
(1–30 seconds, at most eight consecutive attempts). Clicking **Disconnect**
cancels pending reconnect work.

The server sends JSON `SignalRecord` messages containing a durable cursor and
the canonical signal envelope. After a full record is successfully queued for
the dashboard, the adapter stores its cursor in `localStorage` under:

```text
skid-monitor.cloud.cursor.v1:<canonical-client-api-endpoint>:tenant:<tenant-uuid>
```

The adapter validates the complete `<tenant-uuid>.<ticket-uuid>` ticket shape,
uses only its tenant UUID as the storage namespace, and never persists the
ticket itself. Every `SignalRecord` tenant must match that authenticated stream
tenant. The next cloud connection passes that tenant-specific cursor as
`after`, ignores already-applied cursors, and continues from the first newer
record. This cursor contract applies only to `http(s)` cloud mode, never to the
raw `ws(s)` bridge.

Persisted edge and alert state uses the same endpoint-and-tenant namespace.
While cloud authentication is pending no dashboard state is restored or
written; a tenant change clears in-memory signal state before restoring the new
tenant namespace. Legacy endpoint-only cursor and unscoped dashboard keys are
not imported into cloud namespaces, avoiding accidental cross-tenant reuse.
See [`../../docs/cloud-solo-deployment.md`](../../docs/cloud-solo-deployment.md)
for split cloud server configuration.

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
