# skid-monitor Kubernetes/Talos 배포 계획

이 문서는 Linux Kubernetes, 특히 Talos single-node 홈랩 배포만 다룬다. Linux/macOS/Windows native
agent package, service manager, 자동 업데이트 정책은 [agent-continuous-deployment.md](agent-continuous-deployment.md)를
따른다.

## 클러스터 환경

| 항목 | 값 |
|---|---|
| Cluster | `talos` (Talos Linux v1.13.5) |
| 노드 | 단일 control-plane 겸 워커 |
| K8s 버전 | v1.36.1 |
| CNI | Flannel |
| 스토리지 | local-path-provisioner |
| 추가 | KubeVirt 설치됨 |
| LoadBalancer | 없음 (NodePort / Ingress만 가능) |

## 컨테이너 레지스트리

로컬 레지스트리를 Talos 클러스터 내부 Pod로 운용하는 방식은 채택하지 않는다.
단일 노드 환경에서 레지스트리 Pod가 재시작 이미지에 의존하는 닭-달걀 문제가 발생한다.

**결정: 별도 기기(라즈베리파이 등)에 `registry:2` 운용**

- 클러스터 생명주기와 완전히 분리
- Talos machine config에 레지스트리 주소 등록 (`talosctl` 패치)
- 빌드 머신에서 push → 라즈베리파이 레지스트리 → Talos pull

## 이미지 빌드 전략

Kubernetes용 Linux Rust 바이너리를 `x86_64-unknown-linux-musl`로 정적 크로스컴파일 후
scratch/distroless 기반 이미지를 사용한다. 이미지 크기를 최소화하고 Talos containerd와의 호환성을
높인다. 이 전략은 macOS `.pkg` 또는 Windows `.msi` 배포를 대체하지 않는다.

## 컴포넌트별 배포 방식

| 컴포넌트 | 종류 | 이유 |
|---|---|---|
| `skid-monitor-agent` | DaemonSet | Linux host `/proc` 직접 읽음, Kubernetes 노드당 1개 필요 |
| `skid-monitor-migrate` | Job | Cloud PostgreSQL schema를 runtime role과 분리해 적용 |
| `skid-monitor-ingress` | Deployment + Service | Keycloak agent JWT를 검증하고 OTLP를 PostgreSQL에 commit |
| `skid-monitor-client-server` | Deployment + Service | user JWT 기반 tenant query/WebSocket access 제공 |
| `skid-edge-agent` | Deployment | 상태 없는 edge signal/media-provider observer |
| `skid-file-node` | Deployment | 상태 없는 서비스 |
| `skid-compute-advisor` | Deployment | 분석 서비스, 필요 시 HPA 연결 |

### skid-monitor-agent 주의사항

Talos/Linux에서 `/proc` 마운트 접근을 허용하려면 Pod spec에 다음이 필요하다:

```yaml
hostPID: true
securityContext:
  privileged: true  # 또는 세밀한 capabilities 설정
volumes:
  - name: proc
    hostPath:
      path: /proc
```

이 설정은 Linux/Kubernetes agent 전용이다. macOS native agent는 `launchd`, Windows native agent는
Windows Service로 배포하며 `/proc` mount나 `hostPID`를 사용하지 않는다.

## 매니페스트 구조

Kustomize를 사용한다. 단일 노드 홈랩 규모에서 Helm은 과도하고, YAML은 환경별 분기가 어렵다.
아래는 목표 구조이며 현재 저장소에는 아직 `k8s/` manifest가 구현되어 있지 않다.

```text
k8s/
├── base/
│   ├── monitor-agent/      # DaemonSet + ServiceAccount + RBAC
│   ├── monitor-migrate/    # pre-deploy Job, privileged DB role secret
│   ├── monitor-ingress/    # agent-facing OTLP/gRPC Deployment + Service
│   ├── monitor-client/     # user-facing REST/WebSocket Deployment + Service
│   ├── edge-agent/         # Deployment + Service, optional media provider observer
│   ├── file-node/
│   └── compute-advisor/
└── overlays/
    └── skid-server/        # NodePort 번호, 레지스트리 주소 등
```

Cloud split mode의 PostgreSQL/Keycloak, TLS, RLS role, 환경변수와 stream-ticket 설정은
[cloud-solo-deployment.md](cloud-solo-deployment.md)를 따른다. PostgreSQL은 가능하면 managed service를
사용하고, cluster 내부에 둘 때도 application Deployment와 생명주기/volume을 분리한다. ingress와
client server는 서로 다른 Service/audience를 사용하며 같은 public route로 합치지 않는다.
PostgreSQL role/grant, schema 계약, backup/retention 경계와 migration runbook은
[postgresql-components-and-migrations.md](postgresql-components-and-migrations.md)를 따른다.

## 서비스 노출

LoadBalancer가 없으므로 Nginx Ingress Controller를 추가하고 `172.30.2.1`로 직접 접근한다.
NodePort는 임시 테스트 용도로만 사용한다.
