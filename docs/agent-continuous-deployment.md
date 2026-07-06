# Agent Continuous Deployment

이 문서는 `skid-monitor-agent`를 Linux, macOS, Windows native 환경까지 포함해 지속적으로 배포하는
기준을 정의한다. Kubernetes/Talos 전용 배포는 [deployment.md](deployment.md)에 둔다.

## Scope

`skid-monitor-agent`는 host collector/gateway다. 운영 환경에서는 다음 실행 형태를 모두 1급 배포
대상으로 다룬다.

| 대상 | 실행 형태 | 배포 산출물 | 서비스 관리자 | 기본 업데이트 경로 |
| --- | --- | --- | --- | --- |
| Linux Kubernetes | `DaemonSet` 또는 gateway `Deployment` | OCI image | kubelet | Kustomize/GitOps image digest 변경 |
| Linux native | 장기 실행 host service | `.deb`, `.rpm`, `tar.gz` | `systemd` | package repository 또는 관리 도구 |
| macOS native | 장기 실행 host daemon | signed/notarized `.pkg`, universal binary | `launchd` `LaunchDaemon` | MDM/Jamf 또는 signed updater |
| Windows native | Windows Service | signed `.msi`, optional `winget` manifest | Service Control Manager | Intune/SCCM/winget 또는 signed updater |
| Edge appliance | gateway/probe service | 장비별 package 또는 `tar.gz` | systemd/supervisor/vendor init | staged rollout |

Linux container image만 release하는 방식은 충분하지 않다. agent는 client library와 달리 host 권한,
서비스 관리자, 로그 위치, 자동 업데이트 방식이 OS마다 다르다.

## Principles

- protocol과 설정은 OS와 무관해야 한다. 모든 agent는 `Signal`과 OTLP export request를 같은 pipeline에
  태운다.
- host metric sampler는 OS별 adapter로 나누고, wire contract는 `skid-protocol`에 둔다.
- artifact는 immutable version과 commit SHA를 포함하고, 배포는 tag보다 digest 또는 서명된 package
  version을 기준으로 한다.
- 자동 업데이트는 agent process가 자기 binary를 직접 덮어쓰지 않는다. 별도 updater, package manager,
  MDM, GitOps controller가 교체를 수행한다.
- rollout은 canary, cohort, jitter를 둔다. 실패 시 마지막 정상 버전으로 되돌릴 수 있어야 한다.
- serial number, hardware UUID, 사용자 이름 같은 식별자는 host sampler에서 수집하지 않는다.

## CI Gates

공통 gate:

- `cargo test --workspace`
- `cargo fmt`와 `cargo clippy`
- `skid-protocol` frame/protocol compatibility test
- agent config load/validation test
- package metadata에 version, commit SHA, target triple 기록

target별 gate:

| Target | 필수 검증 |
| --- | --- |
| Linux Kubernetes | image build, non-root 또는 제한 권한 검토, `/proc` mount manifest check, smoke pod |
| Linux native | `systemd` unit lint, package install/remove smoke test, `/proc`/filesystem permission check |
| macOS native | `aarch64-apple-darwin`, `x86_64-apple-darwin` build, code signing, notarization, `launchd` load/unload smoke test |
| Windows native | `x86_64-pc-windows-msvc` build, MSI signing, service install/start/stop/uninstall smoke test |

Windows runtime verification은 Windows runner에서만 완료로 본다. macOS command fixture만으로 Windows
support를 완료 상태로 표시하지 않는다.

## OS Sampler Model

Linux는 `/proc`, `/sys`, cgroup, network/disk stat 같은 kernel interface를 우선한다. Kubernetes
배포에서는 host namespace와 mount 권한이 필요할 수 있으므로 manifest와 RBAC를 함께 검증한다.

macOS는 Linux `/proc`가 없으므로 native sampler가 별도로 동작한다. 현재 구현은 `uptime`,
`vm_stat`, `df -k /`, `pmset -g batt` 계열 명령 출력에서 load, VM/memory, filesystem, battery/AC
상태를 만든다. 장기적으로는 `sysctl`, IOKit, `host_statistics` 기반 sampler로 옮길 수 있다.

Windows는 별도 native target으로 둔다. 초기 수집 후보는 PDH/Performance Counters, WMI/CIM,
ETW, Event Log, Service Control Manager 상태다. 구현 전까지 Windows 배포 문서는 planned target으로
표시하고, runtime-verified support로 쓰지 않는다.

source identity는 UI와 downstream exporter가 OS를 구분할 수 있게 해야 한다. 현재 macOS metric은
`skid_monitor.source=macos`로 분리한다. Windows metric source는 구현 PR에서 `skid-protocol`의
정준 값과 함께 고정한다.

## Release Channels

| 채널 | 용도 | 배포 규칙 |
| --- | --- | --- |
| dev | main branch 검증 | short-lived artifact, 내부 테스트만 |
| beta | 제한된 host cohort | 자동 업데이트 가능, canary 비율 제한 |
| stable | 운영 host | 명시 승인 또는 관리 도구 rollout |

agent는 시작 로그와 health/status endpoint에 version, commit SHA, target triple, config path를 남겨야
한다. client-visible health signal을 추가하면 rollout 상태를 dashboard에서 볼 수 있다.

## Kubernetes Rollout

Kubernetes agent는 image digest를 Kustomize overlay에 반영해 배포한다. `latest` tag는 production에서
쓰지 않는다.

권장 순서:

1. CI가 image를 build하고 registry에 push한다.
2. staging overlay의 digest를 갱신한다.
3. smoke signal을 확인한다.
4. production overlay PR을 merge한다.
5. rollout timeout 또는 health failure가 나면 이전 digest로 revert한다.

`skid-monitor-agent`가 host `/proc`를 읽는 DaemonSet이면 privileged 또는 세밀한 capability 설정,
`hostPID`, hostPath mount가 필요할 수 있다. 이 권한 모델은 Linux/Kubernetes 전용이며 macOS/Windows
native agent 배포와 섞지 않는다.

## Native Package Rollout

Linux native package는 전용 user, config directory, log path, `systemd` unit을 함께 설치한다. package
post-install은 서비스를 자동 enable할지 명시 정책을 따라야 하며, uninstall은 config와 data 보존
정책을 분리해야 한다.

macOS package는 code signing과 notarization을 통과해야 한다. `LaunchDaemon` plist는 root daemon이
필요한지, 전용 user로 충분한지, 어떤 command dependency를 요구하는지 명시한다.

Windows package는 MSI signing, Windows Service 등록, Event Log source 등록, firewall rule 필요 여부를
명시한다. service account와 권한은 LocalSystem을 기본값으로 고정하지 말고 최소 권한 계정을 검토한다.

## Automatic Update

자동 업데이트 우선순위는 다음과 같다.

1. GitOps, package repository, MDM, Intune/SCCM 같은 외부 관리 시스템
2. 별도 updater service
3. 수동 package 재설치

자체 updater가 필요할 때의 최소 계약:

- signed update manifest를 받는다.
- artifact hash와 signature를 검증한다.
- versioned install directory에 새 binary를 풀고, atomic switch로 활성 버전을 바꾼다.
- service restart 후 health deadline 안에 정상 신호가 없으면 이전 버전으로 rollback한다.
- cohort와 jitter를 지원해 모든 host가 동시에 재시작하지 않게 한다.
- downgrade는 rollback policy 또는 운영자 승인 없이 허용하지 않는다.

agent process 본체는 update decision과 binary replacement를 직접 수행하지 않는다. 본체는 현재 version,
health, config validation 결과를 노출하고 graceful shutdown을 제공한다.

## Current Gaps

- Linux/Kubernetes 배포 계획은 문서화되어 있으나 실제 `k8s/` manifest 산출물은 아직 없다.
- macOS host sampler는 존재하지만 signed/notarized `.pkg`와 `launchd` installer는 없다.
- Windows native sampler, `Source` 값, MSI packaging, Windows Service installer는 아직 구현되지 않았다.
- `doctor` 또는 `--check` 명령이 아직 없으므로 설치 전 권한/주소/config 검증 UX가 부족하다.
