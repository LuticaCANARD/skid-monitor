# RFC 0001: skid-file-node Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-file-node/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-file-node` |
| Decision Type | File capability node responsibility |

## Abstract

`skid-file-node`는 read-only file root의 capability를 알리고, client-server 간 파일 교신을 안전하게
열기 위한 node다. 현재 구현은 지정된 root를 훑어 availability, file count, total bytes를 metric으로
보내는 단계지만, 제품 범위에는 허용된 root에서 파일 offer를 노출하고 client가 선택한 파일을 chunk로
받는 흐름이 포함된다.

## Responsibilities

- `--root label=/path` 인자를 읽어 관측 root 목록을 만든다.
- root별 available/files/bytes metric을 만든다.
- `node_name`, `root_label`, `root_path` attribute를 붙인다.
- `Source::FileNode`로 OTLP metrics request를 생성한다.
- `SKID_MONITOR_DEVICE_ADDR`로 agent device socket에 `Signal::Metrics`를 보낸다.
- 허용된 root에서 file offer 목록을 만들고, 파일 크기/mtime/hash 같은 download metadata를 제공한다.
- client가 요청한 read-only file을 chunk 단위로 제공하는 file transfer plane에 참여한다.
- transfer 시작/완료/실패/bytes sent metric과 audit event를 남긴다.

## Boundaries

파일 교신은 `skid-monitor-client -> skid-monitor-agent -> skid-file-node` 흐름을 기본으로 한다.
client가 file node에 직접 붙는 공개 endpoint를 여는 것이 아니라, agent/server가 권한 확인과 중계를
맡고 file node는 allowlist root 안의 read-only 파일만 제공한다.

device ingress는 offer/상태 metric을 위한 control/telemetry path다. 실제 file chunk는 device metric
`Signal` payload에 싣지 않고 별도 file transfer plane에서 다룬다. 따라서 file transfer가 포함되더라도
device socket의 16 MiB metric frame을 파일 운반 채널로 남용하지 않는다.

전송 보호는 우선 Tailscale/WireGuard, Kubernetes NetworkPolicy/service mesh, 또는 proxy TLS
termination으로 해결한다. `skid-file-node` 자체 native TLS는 MVP 필수 요구사항이 아니며, public direct
endpoint를 기본값으로 열지 않는다.

권한 경계는 root allowlist, canonical path 검증, symlink policy, per-request authorization, chunk hash,
TTL, audit log를 포함해야 한다.

## Non-Goals

- write access, upload, delete를 제공하지 않는다.
- root allowlist 밖의 filesystem을 탐색하거나 전송하지 않는다.
- file node를 public internet에 직접 노출하는 standalone file server로 만들지 않는다.
- device ingress `Signal::Metrics` frame에 파일 chunk를 싣지 않는다.

## Open Questions

- recursive scan을 도입할지, 현재처럼 top-level file count만 유지할지.
- symlink와 permission error를 metric으로 어떻게 표현할지.
- root label과 credential을 어떻게 binding할지.
- chunk size, resume/range request, hash 검증, compression을 어떤 protocol로 고정할지.
- agent가 file chunk를 proxy할지, 짧은 수명의 signed URL/direct connection을 발급할지.
- Kubernetes sidecar에서 transfer 요청 권한을 Pod identity와 어떻게 연결할지.
