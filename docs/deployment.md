# skid-monitor Kubernetes 배포 계획

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

Rust 바이너리를 `x86_64-unknown-linux-musl`로 정적 크로스컴파일 후 scratch/distroless 기반 이미지를 사용한다.
이미지 크기를 최소화하고 Talos containerd와의 호환성을 높인다.

## 컴포넌트별 배포 방식

| 컴포넌트 | 종류 | 이유 |
|---|---|---|
| `skid-monitor-agent` | DaemonSet | 호스트 `/proc` 직접 읽음, 노드당 1개 필요 |
| `skid-edge-agent` | Deployment | 상태 없는 서비스 |
| `skid-file-node` | Deployment | 상태 없는 서비스 |
| `skid-stream-node` | Deployment | 상태 없는 서비스 |
| `skid-compute-advisor` | Deployment | 분석 서비스, 필요 시 HPA 연결 |

### skid-monitor-agent 주의사항

Talos에서 `/proc` 마운트 접근을 허용하려면 Pod spec에 다음이 필요하다:

```yaml
hostPID: true
securityContext:
  privileged: true  # 또는 세밀한 capabilities 설정
volumes:
  - name: proc
    hostPath:
      path: /proc
```

## 매니페스트 구조

Kustomize를 사용한다. 단일 노드 홈랩 규모에서 Helm은 과도하고, YAML은 환경별 분기가 어렵다.

```text
k8s/
├── base/
│   ├── monitor-agent/      # DaemonSet + ServiceAccount + RBAC
│   ├── edge-agent/         # Deployment + Service
│   ├── file-node/
│   ├── stream-node/
│   └── compute-advisor/
└── overlays/
    └── skid-server/        # NodePort 번호, 레지스트리 주소 등
```

## 서비스 노출

LoadBalancer가 없으므로 Nginx Ingress Controller를 추가하고 `172.30.2.1`로 직접 접근한다.
NodePort는 임시 테스트 용도로만 사용한다.
