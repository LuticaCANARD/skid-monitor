# skid-file-node Use Cases

## 1. Read-Only Log Root Offer

서비스 로그 directory가 존재하고 얼마나 큰지 agent로 알린다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 \
cargo run -p skid-file-node -- --root logs=/var/log/my-service --once
```

## 2. Workload Sidecar File Visibility

Kubernetes Pod나 local workload 옆에 read-only volume을 붙이고, 해당 volume이 monitor plane에 보이는지
확인한다.

## 3. Capacity Snapshot

root별 file count와 total bytes를 주기적으로 보내 storage growth의 coarse signal로 사용한다.

## 4. Future Transfer Readiness Check

download를 열기 전에 어떤 root label과 path가 관측되는지 검증한다. 이 use case는 metadata 확인까지만
허용하며 파일 내용 전송은 포함하지 않는다.
