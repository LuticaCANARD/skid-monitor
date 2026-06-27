# RFC 0005: Stream Telemetry and Media Preview

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `docs/rfcs/0005-stream-telemetry-and-media-preview.md` |
| Scope | future `skid-stream-node`, `skid-monitor-client`, `.NET` extension host, `skid-protocol::metrics` |
| Related | RFC 0001, RFC 0002, RFC 0003, `LuticaCANARD/SKIDStreamPipe` |
| Decision Type | Stream observability, UI boundary, media safety |

## Abstract

`SKIDStreamPipe`에는 HLS/DASH C++ server, Rust camera starter, Svelte/WebRTC client, WebGPU/WASM
renderer의 실험이 있다. 이 RFC는 그중 media bytes를 직접 가져오지 않고, stream 상태와 preview
metadata를 Skid Monitor의 관측 모델로 가져온다.

핵심 결정은 device ingress로 raw video/audio frame을 보내지 않는 것이다. Skid Monitor는 media
transport가 아니라 stream telemetry와 preview coordination plane이다.

## Imported Ideas

`SKIDStreamPipe`에서 가져올 요소:

- WebRTC connection state, ICE, data channel metadata
- `fps`, `bitrate`, `packetsReceived`, resolution 같은 stream stats
- HLS/DASH endpoint와 server status page 개념
- WebGPU/WASM renderer가 metadata overlay를 얹는 구조
- FFmpeg/GStreamer 같은 external media pipeline과의 sidecar 경계

Skid Monitor에 가져오는 것은 status metrics, endpoint metadata, client extension hook이다.

## Decision Summary

- stream telemetry source를 `stream`으로 추가한다.
- future binary 이름은 `skid-stream-node`로 둔다.
- `skid-stream-node`는 media pipeline 옆 sidecar로 배치한다.
- device socket에는 stream metadata와 health metric만 보낸다.
- media URL은 attribute로만 노출한다. raw frame은 보내지 않는다.
- WebGPU renderer는 monitor client core가 아니라 extension/view layer 후보로 둔다.

## Stream Metrics

초기 metric 이름은 다음과 같다.

| Metric | Unit | Attributes |
| --- | --- | --- |
| `stream.source.up` | none | `node_name`, `stream_id`, `protocol` |
| `stream.video.width` | `px` | `stream_id` |
| `stream.video.height` | `px` | `stream_id` |
| `stream.video.fps` | `fps` | `stream_id`, `measured_by` |
| `stream.video.bitrate` | `bit/s` | `stream_id`, `codec` |
| `stream.audio.bitrate` | `bit/s` | `stream_id`, `codec` |
| `stream.packets.received` | none | `stream_id`, `protocol` |
| `stream.packets.lost` | none | `stream_id`, `protocol` |
| `stream.jitter` | `ms` | `stream_id`, `protocol` |
| `stream.rtt` | `ms` | `stream_id`, `protocol` |
| `stream.hls.segment.age` | `ms` | `stream_id`, `endpoint` |
| `stream.dash.manifest.age` | `ms` | `stream_id`, `endpoint` |
| `stream.renderer.frames` | none | `stream_id`, `renderer` |
| `stream.renderer.webgpu.supported` | none | `renderer` |

상태 값은 numeric gauge로 보낸다. 예를 들어 `stream.source.up`은 up이면 1, down이면 0이다.
string 상태는 attributes로 둔다. 예: `connection_state=connected`.

## Endpoint Metadata

media endpoint는 metric attribute로만 전달한다.

```text
stream_id=camera-front
protocol=webrtc
signaling_url=ws://127.0.0.1:8080
preview_url=http://127.0.0.1:8080/status
hls_url=http://127.0.0.1:8080/hls/stream.m3u8
dash_url=http://127.0.0.1:8080/dash/stream.mpd
```

client는 이 URL을 자동 재생하지 않는다. 사람이 명시적으로 preview extension을 켤 때만 사용한다.

## Node Model

`skid-stream-node`의 초기 역할은 세 가지다.

1. WebRTC stats source에서 `RTCStatsReport` 또는 equivalent JSON을 읽는다.
2. HLS/DASH status endpoint를 poll한다.
3. pipeline process health를 관측한다.

예시 설정:

```yaml
interfaces:
  - name: camera-front-stream
    kind: external
    adapter: skid-stream-node
    args:
      stream_id: camera-front
      protocol: webrtc
      signaling_url: ws://127.0.0.1:8080
      status_url: http://127.0.0.1:8080/status
      hls_url: http://127.0.0.1:8080/hls/stream.m3u8
      dash_url: http://127.0.0.1:8080/dash/stream.mpd

bindings:
  - name: camera-front-preview
    kind: stream
    resource_type: webrtc
    interface: camera-front-stream
    capabilities:
      - observe
      - preview_metadata
    control_transport: device-control
```

## Client and Extension Boundary

`skid-monitor-client`의 core console view는 다음만 한다.

- stream id, up/down, protocol, fps, bitrate, resolution을 출력한다.
- endpoint URL이 있음을 표시하되 자동 연결하지 않는다.

WebGPU/WebRTC preview는 extension host나 future GUI client가 담당한다. 이때 `.NET` extension SDK는
stream signal을 받아 별도 viewer를 띄우는 샘플을 제공할 수 있다.

## Safety

- raw media bytes는 device socket에 싣지 않는다.
- media endpoint URL은 민감 정보일 수 있으므로 기본 view에서는 host/path를 축약할 수 있다.
- public URL은 설정에서 `expose_endpoint_attributes=true`가 켜져 있을 때만 attribute로 보낸다.
- signaling URL에 credential이 포함되면 reject하거나 redaction한다.
- stream telemetry node는 pipeline process를 죽이거나 재시작하지 않는다.

## Implementation Plan

1. `skid_protocol::metrics::Source::Stream`을 추가한다.
2. `skid-monitor-client::view`가 `skid_monitor.source=stream` resource를 요약 표시한다.
3. `skid-stream-node` crate를 추가하고 mock stats를 보낸다.
4. WebRTC stats JSON adapter를 붙인다.
5. HLS/DASH status poller를 붙인다.
6. `.NET` sample extension에 stream event handler를 추가한다.
7. future GUI 또는 web dashboard에서 WebGPU preview overlay를 실험한다.

## Non-Goals

- HLS/DASH/WebRTC server를 `skid-monitor-agent` 안에 내장하지 않는다.
- FFmpeg pipeline을 직접 관리하지 않는다.
- browser WebGPU renderer를 Rust console client에 넣지 않는다.
- stream telemetry를 alerting policy로 확정하지 않는다.
