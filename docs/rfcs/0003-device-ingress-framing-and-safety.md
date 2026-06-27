# RFC 0003: Device Ingress Framing and Safety

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `docs/rfcs/0003-device-ingress-framing-and-safety.md` |
| Scope | `skid-protocol`, `skid-monitor-agent::device_socket`, all capability nodes |
| Related | RFC 0001, RFC 0002, `LuticaCANARD/skid-node` tunnel/RPC frame designs |
| Decision Type | Wire format, migration, safety requirements |

## Abstract

현재 device socket은 `u32 length + JSON Signal`만 읽는다. 단순하고 충분히 작지만, accidental protocol
mix-up, 버전 관리, 인증 전 단계 reject, source identity, per-frame metadata를 담을 공간이 없다.

이 RFC는 `skid-node`의 RPC/tunnel frame에서 가져온 원칙을 Skid Monitor용으로 줄여 `skid_device_v1`
frame을 정의한다. migration 기간에는 legacy frame을 계속 받고, 기본 모드는 `auto`로 둔다.

## Decision Summary

- device ingress의 다음 wire format은 `SKDM` magic을 가진 binary envelope다.
- payload는 당분간 기존 `serde_json` 직렬화 `Signal` 그대로 둔다.
- frame header는 source identity와 payload metadata를 별도 JSON header에 둔다.
- bad magic, unsupported version, oversize header/payload는 `Signal` decode 전에 reject한다.
- legacy `u32 length + JSON Signal`은 `SKID_MONITOR_DEVICE_FRAME=legacy|auto|v1` 중 `auto`에서만 허용한다.
- 인증은 v1 header에 slot을 마련하지만, RFC 0003의 첫 구현은 unauthenticated v1이다.

## Frame Layout

모든 정수는 big-endian이다.

```text
0      4      magic         "SKDM"
4      1      version       1
5      1      frame_type    1=signal_json, 2=hello, 3=ping, 4=pong, 5=error
6      2      flags         bitfield, v1에서는 0
8      2      header_len    JSON metadata header bytes
10     2      reserved      0
12     4      payload_len   payload bytes
16     8      frame_id      sender-local monotonically increasing id, 0 allowed
24     N      header        UTF-8 JSON object
24+N   M      payload       frame_type별 payload
```

제한값:

| 항목 | 기본값 |
| --- | ---: |
| `max_header_bytes` | 64 KiB |
| `max_payload_bytes` | 16 MiB |
| `read_timeout` | 5초 |
| `max_connections` | 128 |

`signal_json` payload는 현재의 `Signal` JSON이다. 서버가 frame metadata와 payload 안의 OTLP resource
attribute를 모두 볼 수 있게 되면, metadata는 routing/safety에만 사용하고 source of truth는 OTLP
attribute로 남긴다.

## Header Shape

```json
{
  "node_name": "edge-gateway-1",
  "node_kind": "edge_device",
  "source": "edge_device",
  "content_type": "application/vnd.skid.signal+json",
  "encoding": "identity",
  "sent_unix_nano": 1782537600000000000,
  "auth": {
    "scheme": "none"
  }
}
```

`source`는 `skid_protocol::metrics::Source::as_str()` 값과 맞춘다. `node_kind`는 설정 파일의
node kind와 맞춘다. 인증이 들어오면 `auth.scheme`은 `shared_secret`, `hmac_sha256`, `mtls` 같은
값으로 확장한다.

## Legacy Migration

초기 구현 순서는 다음이다.

1. `device_socket` reader에 `auto` decoder를 추가한다.
2. 처음 4 bytes가 `SKDM`이면 v1 frame으로 읽는다.
3. 그렇지 않으면 기존처럼 첫 4 bytes를 length로 해석한다.
4. sender node들은 기본 legacy를 유지하고 `SKID_MONITOR_DEVICE_FRAME=v1`일 때만 v1로 보낸다.
5. 한 release 뒤 agent 기본값을 `auto`, node 기본값을 `v1`로 바꾼다.
6. legacy는 public bind가 아닌 loopback/trusted LAN에서만 허용한다.

## Safety Requirements

v1 reader는 다음 조건을 만족해야 한다.

- header와 payload allocation 전에 길이 제한을 검사한다.
- frame type이 unknown이면 payload를 읽지 않고 reject한다.
- `header_len` 또는 `payload_len`이 limit보다 크면 reject metric만 남기고 close한다.
- JSON header parse 실패와 Signal parse 실패를 다른 counter로 기록한다.
- accepted frame만 `transport::send`로 forward한다.
- connection별 read timeout을 둔다.
- per-peer concurrent connection cap을 둔다. 구현이 없을 때는 global cap부터 시작한다.
- public listen에서는 v1만 허용한다.

초기 metric 이름:

| Metric | Source | 의미 |
| --- | --- | --- |
| `device_ingress.connections.accepted` | `system` | accepted TCP connection 수 |
| `device_ingress.frames.accepted` | `system` | decode 후 forward된 frame 수 |
| `device_ingress.frames.rejected` | `system` | reject된 frame 수 |
| `device_ingress.frame.bytes` | `system` | payload bytes |
| `device_ingress.frame.header_bytes` | `system` | metadata header bytes |
| `device_ingress.decode.errors` | `system` | Signal JSON decode 오류 |
| `device_ingress.frame.bad_magic` | `system` | magic mismatch |
| `device_ingress.frame.oversize` | `system` | header/payload limit 초과 |

## Compatibility With Client Transport

agent에서 client로 보내는 transport는 당분간 legacy `u32 length + JSON Signal`을 유지한다. device
ingress와 human-facing client stream은 서로 다른 trust boundary다. client transport v1 frame은
별도 RFC로 둔다.

## Implementation Plan

1. `skid-protocol::frame` 또는 `skid-protocol::device_frame`에 header constants와 parser를 둔다.
2. `skid-monitor-agent::device_socket`에 auto decoder를 적용한다.
3. `skid-edge-agent`, `skid-file-node`, `skid-compute-advisor` sender에 v1 writer를 추가한다.
4. RFC 0002 설정의 `device_ingress.protocol`과 `SKID_MONITOR_DEVICE_FRAME`을 연결한다.
5. reject/accept counters를 host metrics에 포함한다.
6. public bind guard를 넣는다. `0.0.0.0`은 `allow_public_ingress`와 v1 frame이 모두 켜져야 허용한다.

## Non-Goals

- 이 RFC만으로 TLS/mTLS를 완성하지 않는다.
- file chunk, media frame, compute payload를 device frame payload로 보내지 않는다.
- exactly-once delivery나 durable queue를 제공하지 않는다.
