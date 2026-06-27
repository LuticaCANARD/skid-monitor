# skid-compute-advisor Product Use Cases

`skid-compute-advisor`는 제품에서 "이 node가 어떤 compute capability를 가졌는가"를 알려주는 advisor다.
작업을 실행하지 않고, 실행을 열 수 있을 만큼 관측과 scoring 표면을 준비한다.

## Kubernetes 운용 호환성

compute advisor는 Kubernetes에서 DaemonSet 또는 workload sidecar로 운용할 수 있다. 다만 이 crate는
scheduler가 아니며, container 안에서 보이는 CPU/memory와 host node 전체 capability는 다를 수 있다.
GPU 판단은 device plugin/DCGM 같은 별도 관측원과의 경계를 명확히 해야 한다.

개발 step:

1. MVP: container cgroup 기준 logical/effective CPU와 memory 정보를 구분해 보낸다.
2. 다음 단계: DaemonSet 배포 시 `k8s.node.name`, namespace, Pod label을 attribute로 붙인다.
3. Production: GPU/device plugin/DCGM 연동은 opt-in으로 두고, RBAC read 권한, resource requests,
   node selector, taint/toleration을 manifest에 명시한다.

## Use Case 1: Node의 기본 Compute Capacity를 본다

제품 경험: 운영자는 site 또는 gateway별 logical CPU 수와 executor 비활성 상태를 client에서 확인한다.
이는 scheduler가 아니라 capacity inventory다.

```sh
SKID_MONITOR_DEVICE_ADDR=127.0.0.1:9101 cargo run -p skid-compute-advisor -- --once
```

개발 step:

1. MVP: logical CPU, GPU 미감지, placeholder score를 보낸다.
2. 다음 단계: memory, cgroup CPU quota, load average를 추가한다.
3. Production: node capability snapshot을 client에서 비교 가능한 inventory view로 렌더링한다.

## Use Case 2: 실행 권한이 없음을 명확히 표시한다

제품 경험: client와 extension은 `executor_enabled=false`를 보고 이 node가 remote job runner가 아니라
관측용 advisor임을 안다.

개발 step:

1. MVP: 모든 compute metric에 `executor_enabled=false` attribute를 붙인다.
2. 다음 단계: UI에서 executor disabled badge를 표시한다.
3. Production: executor를 여는 future RFC가 생기면 credential, audit, quota가 없으면 enabled가 될 수
   없게 한다.

## Use Case 3: Route Advice 후보를 제품 화면에 올린다

제품 경험: 사용자는 workload를 어디로 보낼지 결정하기 전에 node별 score, confidence, reason을 본다.
현재는 placeholder지만 제품 화면 형태를 먼저 검증한다.

개발 step:

1. MVP: `compute_advisor.route.score.placeholder`를 표시한다.
2. 다음 단계: `compute_advisor.route.score`, `confidence`, `reason` attribute로 승격한다.
3. Production: measured input이 부족할 때 conservative default와 confidence policy를 문서화한다.

## Use Case 4: Opt-In Workload Probe로 실제 성능 힌트를 얻는다

제품 경험: GPU/image/encoding probe를 사용자가 명시적으로 켠 node만 synthetic workload를 실행해
capacity hint를 보낸다.

개발 step:

1. MVP: dependency 없는 memory bandwidth/noop probe를 추가한다.
2. 다음 단계: external adapter allowlist, timeout, bad output metric을 구현한다.
3. Production: probe profile, safety budget, GPU memory failure handling을 config와 UI에 노출한다.
