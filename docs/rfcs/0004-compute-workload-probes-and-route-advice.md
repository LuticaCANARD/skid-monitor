# RFC 0004: Compute Workload Probes and Route Advice

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `docs/rfcs/0004-compute-workload-probes-and-route-advice.md` |
| Scope | `skid-compute-advisor`, `skid-protocol::metrics`, future workload adapters |
| Related | RFC 0001, RFC 0002, `LuticaCANARD/SKID-rust`, `LuticaCANARD/LuticaSKID` |
| Decision Type | Capability model, workload probes, scoring |

## Abstract

`skid-compute-advisor`는 현재 logical CPU 수와 placeholder score만 보낸다. 이 RFC는
`SKID-rust`와 legacy `.NET/F# LuticaSKID`에서 가져올 수 있는 GPU/image processing workload를
직접 이식하지 않고, compute capability를 검증하는 probe와 route advice 입력값으로 정의한다.

핵심 원칙은 "실행기는 아직 열지 않되, 실행할 수 있는 모양은 관측한다"이다.

## Imported Ideas

`SKID-rust`에서 가져올 요소:

- WGPU/CUDA/HIP 같은 backend label
- normal map generation, resize, procedural image generation 같은 GPU workload
- Rust native library와 C#/JVM binding을 분리한 경계
- 성능 문서의 경고: image memory layout, lock contention, resize scaledown 미완성 같은 이슈

`LuticaSKID`에서 가져올 요소:

- normal map, histogram, color grouping, color mood 같은 image workload taxonomy
- Unity 호환 `.NET` extension 가능성
- ILGPU 기반 accelerator 개념

Skid Monitor에 가져오는 것은 구현체 전체가 아니라 workload 이름, capability label, probe metric이다.

## Decision Summary

- `skid-compute-advisor`는 계속 별도 binary다.
- 기본 모드는 passive capability report다.
- synthetic workload probe는 opt-in이다.
- GPU/image library를 workspace에 직접 vendoring하지 않는다.
- probe runner는 처음에는 builtin mock과 external command 두 종류만 둔다.
- route advice는 "권고 metric"으로만 보낸다. 원격 실행 예약이나 dispatch는 하지 않는다.

## Capability Metrics

기본 metric은 다음을 목표로 한다.

| Metric | Unit | Attributes |
| --- | --- | --- |
| `compute_advisor.parallelism.logical_cpus` | none | `node_name` |
| `compute_advisor.parallelism.effective_cpus` | none | `node_name`, `cgroup_limited` |
| `compute_advisor.memory.total` | `By` | `node_name` |
| `compute_advisor.memory.available` | `By` | `node_name` |
| `compute_advisor.gpu.detected` | none | `node_name`, `backend` |
| `compute_advisor.gpu.count` | none | `node_name`, `backend` |
| `compute_advisor.gpu.vram.total` | `By` | `node_name`, `backend`, `device_name` |
| `compute_advisor.thermal.temperature` | `C` | `node_name`, `device` |
| `compute_advisor.network.rtt` | `ms` | `node_name`, `target` |
| `compute_advisor.route.score` | none | `node_name`, `workload`, `confidence` |

현재 구현된 `compute_advisor.gpu.detected=0`과 `route.score.placeholder`는 이 RFC의 migration seed로
간주한다.

## Workload Probe Taxonomy

첫 probe 이름은 다음처럼 둔다.

| Probe | Origin | 의미 |
| --- | --- | --- |
| `image.normal_map.256` | `SKID-rust`, `LuticaSKID` | 작은 heightmap에서 normal map 생성 |
| `image.resize.512_to_1024` | `SKID-rust`, `LuticaSKID` | bilinear resize |
| `image.histogram.rgb_64` | `LuticaSKID` | RGB histogram 생성 |
| `image.color_mood.kmeans` | `LuticaSKID` | dominant color 기반 color mapping |
| `stream.encode.h264_720p` | `SKIDStreamPipe` | media encode capability hint |
| `compute.noop.memory_bandwidth` | builtin | dependency 없는 memory copy baseline |

probe metric 이름:

| Metric | Unit | Attributes |
| --- | --- | --- |
| `compute_probe.duration` | `ms` | `probe`, `backend`, `status` |
| `compute_probe.throughput` | `items/s` | `probe`, `backend` |
| `compute_probe.error` | none | `probe`, `backend`, `error_kind` |
| `compute_probe.enabled` | none | `probe` |

## Route Advice

초기 score는 설명 가능한 가중합으로 둔다.

```text
score =
  capacity_score * 0.35 +
  current_load_score * 0.25 +
  thermal_score * 0.15 +
  network_score * 0.15 +
  data_locality_score * 0.10
```

각 항목은 0.0부터 1.0까지다. 값이 없으면 confidence를 낮추고 score에는 conservative default를 넣는다.

필수 attributes:

- `node_name`
- `workload`
- `backend`
- `executor_enabled=false`
- `confidence`
- `reason`

`reason`은 사람이 읽는 짧은 label이다. 예: `cpu_only`, `gpu_present_probe_disabled`,
`thermal_limited`, `network_unknown`, `file_locality_match`.

## External Workload Adapter

향후 `SKID-rust`나 `.NET LuticaSKID`를 실제로 호출할 때도 advisor는 직접 library ABI에 묶이지 않는다.
초기 adapter는 command protocol로 둔다.

```json
{
  "probe": "image.normal_map.256",
  "backend": "wgpu",
  "input": {
    "width": 256,
    "height": 256,
    "format": "rgba_f32"
  }
}
```

adapter는 다음 JSON을 stdout에 쓴다.

```json
{
  "status": "ok",
  "duration_ms": 3.4,
  "throughput_items_per_second": 19275.0,
  "backend": "wgpu",
  "device_name": "adapter-0"
}
```

timeout, stderr, non-zero exit는 `compute_probe.error`로 변환한다.

## Safety

- synthetic probe는 기본 off다.
- probe timeout 기본값은 2초다.
- probe input size는 설정에서 상한을 둔다.
- GPU memory allocation failure는 advisor 프로세스를 죽이지 않고 error metric으로 바꾼다.
- external command adapter는 allowlist path만 실행한다.
- route advice는 실행 권한이 아니다.

## Implementation Plan

1. `skid-compute-advisor` config에 `--probe`, `--probe-once`, `--probe-timeout-ms`를 추가한다.
2. dependency 없는 CPU/memory builtin probe부터 넣는다.
3. Linux에서 `/proc/meminfo`, cgroup CPU quota, thermal zone을 읽는다.
4. GPU detection은 처음에 environment/manual labels를 지원한다. 실제 WGPU/CUDA enumeration은 feature flag로 둔다.
5. external command adapter를 추가하고 `SKID-rust` normal map probe를 별도 binary로 실험한다.
6. route score를 placeholder에서 `compute_advisor.route.score`로 승격한다.
7. client view는 `workload`, `score`, `confidence`, `reason`을 한 줄로 렌더링한다.

## Non-Goals

- remote execution API를 열지 않는다.
- `SKID-rust` 또는 `LuticaSKID` 코드를 통째로 workspace에 복사하지 않는다.
- CUDA/WGPU/HIP dependency를 기본 build에 강제하지 않는다.
- scoring을 scheduler decision으로 사용하지 않는다.
