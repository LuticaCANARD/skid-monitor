# skid-file-node Product Use Cases

`skid-file-node`는 제품에서 "어떤 파일 root가 관측 가능하고, 필요한 파일을 안전하게 받을 수 있는지"를
알려주는 capability node다. 현재 구현은 root metadata metric까지만 보내지만, 제품 use case에는
client-server 간 read-only 파일 교신이 포함된다.

## Kubernetes 운용 호환성

file node는 Kubernetes와 잘 맞는 sidecar다. workload Pod에 read-only volume을 붙이고 같은 Pod 안에서
agent loopback ingress로 offer/상태 metric을 보내는 형태를 기본 호환 경로로 둔다. read-only file
download를 열 때도 file node를 public Service로 직접 노출하지 않고 agent/server의 권한 확인과 중계를
통과하게 한다. Pod 밖 전송이 필요하면 Tailscale/WireGuard, service mesh mTLS, 또는 proxy 계층에서
보호한다. host 전체를 `hostPath`로 여는 운용은 기본 use case가 아니다.

개발 step:

1. MVP: 명시된 `--root label=/path`만 관측하고 read-only volume sidecar 예시를 만든다.
2. 다음 단계: namespace, Pod, workload, container label을 metric attribute와 file offer metadata에
   붙일 수 있게 한다.
3. Production: root allowlist, scan budget, symlink policy, PodSecurity restricted profile,
   `automountServiceAccountToken=false`, transfer request authorization을 manifest 기준으로 고정한다.

## Use Case 1: 운영 로그 Root의 존재와 크기를 본다

제품 경험: 운영자는 특정 서비스의 log directory가 존재하는지, 파일 수와 총 bytes가 얼마나 되는지
client에서 확인한다. 장애 조사 전에 "로그가 쌓이고 있는가"를 빠르게 본다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 \
cargo run -p skid-file-node -- --root logs=/var/log/my-service --once
```

개발 step:

1. MVP: top-level file count와 total bytes를 보낸다.
2. 다음 단계: permission denied, missing root, symlink skipped 같은 상태 metric을 추가한다.
3. Production: recursive scan policy, max scan budget, root allowlist를 config로 고정한다.

## Use Case 2: Workload Sidecar가 Read-Only Volume을 알리고 파일을 제공한다

제품 경험: Kubernetes Pod나 local service 옆에 file node를 붙여, workload가 가진 read-only artifact나
log volume의 상태를 monitor plane에 노출한다. 사용자는 client에서 offer를 보고 허용된 파일만
download한다.

개발 step:

1. MVP: `--root label=/path`로 명시된 root만 관측한다.
2. 다음 단계: same-Pod sidecar manifest와 read-only mount 예시를 제공하고 file offer 목록을 만든다.
3. Production: Pod identity, namespace, workload label을 metric/offer/transfer audit attribute로 붙인다.

## Use Case 3: Storage Growth를 조기 감지한다

제품 경험: root별 bytes 증가를 client나 extension이 추적해 disk pressure의 초기 징후로 사용한다.

개발 step:

1. MVP: `file_node.root.bytes` gauge를 주기적으로 보낸다.
2. 다음 단계: client extension에서 growth rate를 계산한다.
3. Production: threshold policy와 retention-aware summary를 추가한다.

## Use Case 4: Client가 필요한 파일을 안전하게 내려받는다

제품 경험: client는 file offer 목록에서 로그, report, artifact를 선택하고 server를 통해 read-only
chunk download를 수행한다. file node는 root allowlist 안의 파일만 읽고, agent/server는 권한 확인,
audit, rate limit, transfer 상태를 담당한다.

개발 step:

1. MVP: root label, path, availability와 file offer metadata(size, mtime, content hash 후보)를 보낸다.
2. 다음 단계: canonical path, symlink policy, range/chunk request, transfer bytes metric을 추가한다.
3. Production: auth, audit, chunk hash 검증, resume, TTL, per-user/per-node rate limit을 file transfer
   plane에 고정한다.

## Use Case 5: 장애 조사용 Support Bundle을 교신한다

제품 경험: 운영자가 client에서 "support bundle"을 요청하면 server가 허용된 root의 로그/상태 파일을
묶어 받는다. 사용자는 Pod나 gateway에 직접 SSH로 들어가지 않고도 필요한 파일을 얻는다.

개발 step:

1. MVP: 단일 파일 download만 지원하고 bundle은 client-side 목록으로 표현한다.
2. 다음 단계: server가 여러 file offer를 하나의 transfer session으로 묶는다.
3. Production: bundle manifest, redaction, maximum bytes, expiration, audit trail을 제공한다.
