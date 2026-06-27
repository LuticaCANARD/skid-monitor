# RFC 0001: skid-compute-advisor Crate Role

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-compute-advisor/docs/rfcs/0001-crate-role.md` |
| Scope | `skid-compute-advisor` |
| Decision Type | Compute capability node responsibility |

## Abstract

`skid-compute-advisor`는 remote executor가 아니라 compute capability를 관측하는 node다. 현재 구현은
logical CPU 수, GPU 미감지 값, route score placeholder를 metric으로 보낸다.

## Responsibilities

- `compute_advisor.parallelism.logical_cpus`를 수집한다.
- `compute_advisor.gpu.detected=0`으로 현재 GPU detection이 없음을 표시한다.
- `compute_advisor.route.score.placeholder`를 보내 future scoring surface를 남긴다.
- 모든 metric에 `node_name`과 `executor_enabled=false` attribute를 붙인다.
- `SKID_MONITOR_DEVICE_ADDR`로 agent device socket에 `Signal::Metrics`를 보낸다.

## Boundaries

이 crate는 compute를 실행하지 않는다. GPU/image workload probe, external adapter, route score 공식은
future implementation surface이며 opt-in이어야 한다.

## Non-Goals

- remote job dispatch API를 열지 않는다.
- Kubernetes scheduler나 batch executor가 되지 않는다.
- CUDA/WGPU/HIP dependency를 기본 build에 강제하지 않는다.

## Open Questions

- placeholder metric을 언제 `compute_advisor.route.score`로 승격할지.
- memory/cgroup/thermal/GPU detection을 어떤 순서로 추가할지.
- probe adapter allowlist와 timeout 정책을 어디까지 crate local config로 둘지.
