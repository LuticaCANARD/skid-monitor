# RFC 0001: skid-file-node Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-file-node/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-file-node` |
| Decision Type | File capability node responsibility |

## Abstract

`skid-file-node`는 file transfer service가 아니라 read-only file root의 capability를 알리는 node다.
지정된 root를 훑어 availability, file count, total bytes를 metric으로 보낸다.

## Responsibilities

- `--root label=/path` 인자를 읽어 관측 root 목록을 만든다.
- root별 available/files/bytes metric을 만든다.
- `node_name`, `root_label`, `root_path` attribute를 붙인다.
- `Source::FileNode`로 OTLP metrics request를 생성한다.
- `SKID_MONITOR_DEVICE_ADDR`로 agent device socket에 `Signal::Metrics`를 보낸다.

## Boundaries

이 crate는 파일 존재와 크기 정보를 관측할 뿐 파일 내용을 전송하지 않는다. symlink policy,
canonical path, hash, TTL, chunk download, authorization은 future file transfer RFC가 필요하다.

## Non-Goals

- 파일 다운로드 API를 열지 않는다.
- write access, upload, delete를 제공하지 않는다.
- root allowlist 밖의 filesystem을 탐색하지 않는다.

## Open Questions

- recursive scan을 도입할지, 현재처럼 top-level file count만 유지할지.
- symlink와 permission error를 metric으로 어떻게 표현할지.
- future transfer plane에서 root label과 credential을 어떻게 binding할지.
