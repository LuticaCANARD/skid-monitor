# RFC 0002: Extensible Edge Media Provider Contract

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-07-02 |
| File | `docs/rfcs/0002-extensible-media-provider.md` |
| Scope | `skid-protocol`, `skid-edge-agent`, `skid-monitor-agent`, `skid-monitor-client`, extension/GUI layer |
| Related | RFC 0001 stream telemetry, component/extension registry |

## Abstract

카메라, 화면 캡처, RTSP gateway, WebRTC relay, HLS/DASH packager, HTTP snapshot server처럼 현장 장비
가까이에서 media bytes를 소유한 구성요소를 `skid-edge-agent`가 확장 provider로 연결할 수 있게 하는 계약을
정의한다.

핵심 결정은 카메라/image/video provider를 별도 `skid-stream-node`가 아니라 `skid-edge-agent`의 edge
provider adapter로 다루는 것이다. provider는 실제 image/video/audio endpoint와 capture lifecycle을
소유하고, Skid Monitor control plane에는 provider identity, media health, preview capability, redacted
endpoint metadata만 올린다. raw image/video/audio bytes는 device ingress `Signal`로 보내지 않는다.

## Goals

- camera image, live video, stream relay, snapshot server를 `skid-edge-agent`의 같은 확장 모델로 다룬다.
- provider별 SDK나 codec stack을 `skid-monitor-agent`와 `skid-monitor-client` core에 vendoring하지 않는다.
- device ingress는 telemetry/control metadata 전용이라는 RFC 0001 원칙을 유지한다.
- 사용자가 명시적으로 preview/download/snapshot action을 선택하기 전까지 endpoint에 자동 연결하지 않는다.
- URL, token, camera location 같은 민감 정보가 기본 console output이나 extension event에 새지 않게 한다.

## Non-Goals

- Skid Monitor core가 media server, transcoder, recorder가 되는 것은 목표가 아니다.
- device ingress frame에 JPEG/PNG/H.264/PCM payload를 싣지 않는다.
- 초기 계약에서 provider별 full control API, PTZ, camera setting mutation, recording retention policy를 열지 않는다.
- 처음부터 typed `Signal::MediaProvider`를 추가하지 않는다. 현재 wire compatibility를 위해 OTLP metrics와
  attributes 위에 descriptor를 표현하고, typed signal은 후속 RFC로 둔다.

## Terms

| Term | Meaning |
| --- | --- |
| media provider | 실제 media endpoint와 capture lifecycle을 소유한 외부 서비스, sidecar, process, device bridge |
| provider adapter | provider를 probe하고 Skid Monitor control metadata로 변환하는 작은 adapter |
| `skid-edge-agent` | edge physical telemetry와 현장 media provider metadata를 agent device ingress로 보내는 binary |
| specialized stream node | edge-agent로 충분하지 않은 relay/packager 전용 구성이 필요할 때의 future adapter option |
| stream endpoint | WebRTC signaling URL, HLS manifest, DASH manifest, RTSP URL, MJPEG URL, snapshot HTTP URL 같은 data-plane 접점 |
| endpoint ref | 실제 URL 대신 control plane에 노출하는 stable logical identifier |
| preview consumer | C# extension, future GUI, WebGPU/WebRTC preview layer처럼 사용자가 켠 뒤 media endpoint에 접속하는 구성요소 |

## Architecture

```text
+----------------------+       control metrics        +------------------------+       Signal        +---------------------+
| media provider       |<---------------------------->| skid-edge-agent        |-------------------->| skid-monitor-agent  |
| owns media bytes     | provider status / endpoints  | provider adapter host  | media metadata      | device ingress      |
+----------------------+                              +------------------------+                    +----------+----------+
                                                                                                             |
                                                                                                             | Signal
                                                                                                             v
                                                                                                  +---------------------+
                                                                                                  | skid-monitor-client |
                                                                                                  | text summary        |
                                                                                                  +----------+----------+
                                                                                                             |
                                                                                                             | explicit user action
                                                                                                             v
                                                                                                  +---------------------+
                                                                                                  | extension / GUI     |
                                                                                                  | preview consumer    |
                                                                                                  +----------+----------+
                                                                                                             |
                                                                                                             | data plane
                                                                                                             v
                                                                                                  +---------------------+
                                                                                                  | media provider      |
                                                                                                  | WebRTC/HLS/snapshot |
                                                                                                  +---------------------+
```

`skid-edge-agent`는 media bytes를 forwarding하지 않는다. 기본 동작은 provider 상태를 probe하고,
agent device ingress에 `Signal::Metrics`를 push하는 것이다. preview consumer는 사용자 동작, permission
check, endpoint redaction 정책을 통과한 뒤 provider의 data-plane endpoint에 직접 접속한다.

agent가 media endpoint를 broker할 수 있는 future mode는 허용하지만 기본값은 아니다. broker mode를 열 경우에도
control plane과 data plane은 분리하고, audit id, auth token issuance, rate limit, connection cap이 필요하다.

`skid-stream-node`라는 별도 binary가 필요해지는 경우는 edge-agent가 camera gateway 근처에서 처리하기 어려운
전문 media relay, packaging, transcoding topology에 한정한다. camera에서 이미지를 가져오는 기본 흐름은
`skid-edge-agent` 소관이다.

## Provider Identity

provider와 stream은 분리해 식별한다. 하나의 provider가 여러 stream을 노출할 수 있고, 하나의 stream이 여러
endpoint protocol을 가질 수 있기 때문이다.

| Field | Required | Example | Notes |
| --- | --- | --- | --- |
| `provider_id` | yes | `lab-camera-gateway-1` | site 안에서 stable해야 한다. |
| `provider_kind` | yes | `webrtc_relay`, `rtsp_bridge`, `snapshot_http`, `mock` | 구현/연결 방식. |
| `node_name` | yes | `lab-gateway-1` | Skid node identity. |
| `stream_id` | yes | `camera-front` | 사용자가 보는 logical stream identity. |
| `media_kind` | yes | `video`, `image`, `audio`, `av` | still image provider는 `image`를 쓴다. |
| `protocol` | yes | `webrtc`, `hls`, `dash`, `rtsp`, `mjpeg`, `snapshot_http` | endpoint별로 다를 수 있다. |
| `endpoint_ref` | no | `preview`, `hls-main`, `latest-snapshot` | URL을 직접 노출하지 않는 stable reference. |

`provider_id`와 `stream_id`는 high-cardinality가 될 수 있으므로 metric cardinality 정책을 따라야 한다. 임의
frame id, request id, token, full URL은 metric attribute로 넣지 않는다.

## Capabilities

provider adapter는 capability를 boolean gauge 또는 descriptor attribute로 노출한다. capability는 "할 수 있음"의
광고이지, 즉시 연결하라는 명령이 아니다.

| Capability | Meaning |
| --- | --- |
| `observe` | 상태, fps, resolution, latency 같은 telemetry를 낼 수 있다. |
| `live_preview` | live preview endpoint가 있다. |
| `snapshot` | 현재 또는 최근 still image를 요청할 수 있다. |
| `manifest_preview` | HLS/DASH manifest를 제공한다. |
| `relay_required` | consumer가 provider에 직접 붙지 않고 broker/relay를 거쳐야 한다. |
| `auth_required` | preview/snapshot 전에 token, session, allowlist가 필요하다. |

초기 구현은 capability별 metric을 많이 만들기보다 `stream.provider.capability` gauge를 사용하고
`capability=<name>` attribute로 표현한다.

## Metrics

RFC 0001의 `stream.*` namespace를 유지하되, producer binary는 `skid-edge-agent`일 수 있다. still image도
raw camera frame이 아니라 media provider 상태로 다루며, image-specific 상태는 `stream.snapshot.*` 아래에 둔다.

| Metric | Kind | Unit | Attributes |
| --- | --- | --- | --- |
| `stream.provider.up` | gauge | none | `node_name`, `provider_id`, `provider_kind` |
| `stream.provider.capability` | gauge | none | `provider_id`, `stream_id`, `capability` |
| `stream.source.up` | gauge | none | `node_name`, `provider_id`, `stream_id`, `protocol` |
| `stream.video.width` | gauge | `px` | `node_name`, `provider_id`, `stream_id` |
| `stream.video.height` | gauge | `px` | `node_name`, `provider_id`, `stream_id` |
| `stream.video.fps` | gauge | `fps` | `node_name`, `provider_id`, `stream_id`, `measured_by` |
| `stream.video.bitrate` | gauge | `bit/s` | `node_name`, `provider_id`, `stream_id`, `codec` |
| `stream.snapshot.available` | gauge | none | `node_name`, `provider_id`, `stream_id`, `endpoint_ref` |
| `stream.snapshot.age` | gauge | `ms` | `node_name`, `provider_id`, `stream_id`, `endpoint_ref` |
| `stream.snapshot.width` | gauge | `px` | `node_name`, `provider_id`, `stream_id` |
| `stream.snapshot.height` | gauge | `px` | `node_name`, `provider_id`, `stream_id` |
| `stream.endpoint.present` | gauge | none | `provider_id`, `stream_id`, `endpoint_ref`, `protocol`, `endpoint_kind` |
| `stream.endpoint.health` | gauge | none | `provider_id`, `stream_id`, `endpoint_ref`, `protocol` |
| `stream.packets.lost` | sum | none | `node_name`, `provider_id`, `stream_id`, `protocol` |
| `stream.jitter` | gauge | `ms` | `node_name`, `provider_id`, `stream_id`, `protocol` |
| `stream.rtt` | gauge | `ms` | `node_name`, `provider_id`, `stream_id`, `protocol` |

`stream.endpoint.present=1`은 endpoint가 있다는 사실만 말한다. endpoint URL은 기본적으로 metric attribute에 넣지
않는다. 운영자가 명시적으로 `expose_endpoint_attributes=redacted`를 켠 경우에만 redacted URL attribute를 붙일
수 있다.

## Endpoint Metadata

control plane에 endpoint를 표현할 때는 실제 URL보다 `endpoint_ref`를 우선한다.

```text
provider_id=lab-camera-gateway-1
provider_kind=webrtc_relay
stream_id=camera-front
media_kind=video
endpoint_ref=preview
endpoint_kind=live_preview
protocol=webrtc
endpoint_present=1
```

URL 노출이 필요한 경우에도 다음 규칙을 따른다.

- `userinfo`가 있는 URL은 reject한다.
- query token, password, session id로 보이는 값은 redact한다.
- public host URL은 `security.allow_public_ingress=true`와 provider allowlist 없이는 노출하지 않는다.
- console client는 endpoint URL을 자동 연결하거나 자동 재생하지 않는다.
- extension/GUI는 permission prompt 또는 manifest permission을 통과해야 endpoint를 resolve한다.

endpoint resolution은 future registry API로 분리한다. 즉 client summary는 `endpoint_ref=preview`를 보여주고,
preview consumer만 "이 ref를 실제 URL로 resolve해도 되는가"를 묻는다.

## Configuration Shape

RFC 0001의 `interfaces`와 `bindings` 모델을 확장한다. provider별 설정은 `interface.args.provider` 아래에 둔다.

```yaml
interfaces:
  - name: front-camera-provider
    kind: external
    adapter: skid-edge-agent
    args:
      provider:
        id: lab-camera-gateway-1
        kind: webrtc_relay
        discovery: static
        streams:
          - id: camera-front
            media_kind: video
            endpoints:
              - ref: preview
                kind: live_preview
                protocol: webrtc
                url: ws://127.0.0.1:8080/signaling
              - ref: latest-snapshot
                kind: snapshot
                protocol: snapshot_http
                url: http://127.0.0.1:8080/snapshot/latest.jpg

bindings:
  - name: front-camera-preview
    kind: stream
    resource_type: webrtc
    interface: front-camera-provider
    capabilities:
      - observe
      - preview_metadata
      - snapshot_metadata
    control_transport: device-control
```

이 설정은 provider endpoint를 선언하지만, client가 자동으로 접속한다는 뜻이 아니다. `bindings[].kind=stream`에서
`data_transport`가 없으면 metadata-only stream으로 취급한다는 RFC 0001 규칙을 유지한다.

## Adapter Modes

초기 `skid-edge-agent`의 media provider layer는 다음 adapter mode를 가질 수 있다.

| Mode | Meaning | Raw bytes handling |
| --- | --- | --- |
| `mock` | demo/test용 synthetic stream metrics | 없음 |
| `status_http` | provider의 JSON status endpoint를 poll | 없음 |
| `webrtc_relay` | signaling endpoint 존재와 health만 관측 | provider 소유 |
| `hls_manifest` | manifest age, segment age, bitrate hint 관측 | provider 소유 |
| `snapshot_http` | latest snapshot의 존재, age, dimensions, content length 관측 | provider 소유 |
| `rtsp_probe` | RTSP source liveness와 codec/resolution/fps hint 관측 | provider 소유 |

provider-specific SDK가 필요한 경우에도 core agent/client가 아니라 별도 adapter crate나 sidecar process에 둔다.

## Still Image Flow

카메라에서 이미지를 가져오는 use case는 live stream과 같은 provider 모델을 사용한다.

1. media provider가 `latest-snapshot` endpoint를 소유한다.
2. `skid-edge-agent`는 endpoint를 probe하고 `stream.snapshot.available`, `stream.snapshot.age`,
   `stream.snapshot.width`, `stream.snapshot.height`만 보낸다.
3. client는 snapshot 가능 여부와 freshness를 표시한다.
4. 사용자가 snapshot preview를 선택하면 extension/GUI가 permission을 확인한다.
5. permission을 통과한 preview consumer가 `endpoint_ref=latest-snapshot`을 resolve하고 data plane으로 이미지를 받는다.

이 흐름에서 JPEG/PNG bytes는 `Signal::Metrics`, `Signal::Logs`, `Signal::Traces`에 들어가지 않는다.

## Client Behavior

core console client는 provider와 stream을 요약한다.

```text
stream camera-front provider=lab-camera-gateway-1 up protocol=webrtc 1280x720 29.8fps endpoints=preview,latest-snapshot
```

URL은 기본적으로 출력하지 않는다. endpoint가 존재한다는 사실, protocol, redacted host 정도만 보여준다. preview는
extension/GUI가 담당하고, extension event에는 raw URL 대신 `provider_id`, `stream_id`, `endpoint_ref`,
`endpoint_kind`, `permission_required`를 우선 전달한다.

## Security Requirements

- media provider adapter는 raw URL, token, frame id, object key를 high-cardinality metric attribute로 넣지 않는다.
- endpoint URL의 userinfo는 reject한다.
- endpoint URL query는 기본 redact한다.
- preview/snapshot action은 audit 가능한 user action이어야 한다.
- public endpoint exposure는 explicit allowlist와 auth policy가 있어야 한다.
- `skid-monitor-agent`는 기본적으로 media bytes를 relay하지 않는다.
- extension/GUI crash가 provider나 core monitor process를 죽이지 않도록 out-of-process boundary를 유지한다.

## Implementation Path

1. `skid-protocol::metrics::Source::Stream`을 추가하고 `as_str() = "stream"`으로 고정한다.
2. `skid-edge-agent`에 media provider adapter module을 추가한다. 첫 mode는 `mock` 또는 `status_http`로 둔다.
3. `stream.provider.*`, `stream.endpoint.*`, `stream.snapshot.*` metric helper를 추가한다.
4. client는 `skid_monitor.source=stream`을 source별 summary로 표시하고 unknown source fallback을 유지한다.
5. endpoint redaction helper와 tests를 client/edge-agent 경계 중 한 곳에 둔다.
6. extension/GUI preview permission contract를 component registry와 연결한다.
7. 필요해지면 후속 RFC에서 typed `Signal::MediaProviderDescriptor` 또는 endpoint resolution API를 정의한다.

## Open Questions

- endpoint resolution API를 agent가 중재할지, extension registry가 로컬 설정으로 해결할지.
- provider descriptor를 계속 OTLP metrics attributes로 유지할지, typed signal로 승격할지.
- `snapshot_http`에서 content length와 etag를 metadata로 노출할 때 privacy/caching 경계를 어디에 둘지.
- WebRTC signaling URL과 HLS/DASH manifest URL의 redaction 정책을 같은 규칙으로 충분히 처리할 수 있는지.
