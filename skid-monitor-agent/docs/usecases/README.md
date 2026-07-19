# skid-monitor-agent Product Use Cases

`skid-monitor-agent`는 제품에서 현장 또는 서버 옆에 설치되는 gateway다. 사용자는 agent 자체를 보는
것보다, agent가 local system과 capability node 신호를 모아 client로 안정적으로 흘려주는 경험을
원한다.

## Kubernetes 운용 호환성

agent는 Kubernetes에서 same-Pod sidecar, site gateway Deployment, cluster-internal collector로 운용될
수 있다. 기본 호환 경계는 Pod 안 loopback ingress이고, Pod 밖으로 device socket을 열 때는 ClusterIP,
NetworkPolicy, overlay VPN 또는 service mesh, authentication, connection limit이 같이 필요하다.

개발 step:

1. MVP: same-Pod sidecar에서 `127.0.0.1:9101`만 열어 file/compute node를 받는다.
2. 다음 단계: readiness/liveness endpoint, graceful shutdown, Pod restart 중 drop policy를 문서화한다.
3. Production: ClusterIP ingress에는 NetworkPolicy, restricted security context, Tailscale/WireGuard
   또는 service mesh mTLS, device enrollment, resource requests/limits를 함께 제공한다.

## Native OS 운용 호환성

agent는 Linux 서버만이 아니라 macOS와 Windows host에도 native service로 배포되는 것을 목표로 한다.
Kubernetes `DaemonSet`은 Linux/Kubernetes 환경의 한 배포 형태일 뿐이며, desktop/workstation/server
host에서는 각 OS의 서비스 관리자와 package 형식을 따른다.

| OS | 실행 관리자 | package 목표 | host metric 경계 |
| --- | --- | --- | --- |
| Linux | `systemd` | `.deb`, `.rpm`, `tar.gz` | `/proc`, `/sys`, cgroup, filesystem/network stats |
| macOS | `launchd` `LaunchDaemon` | signed/notarized `.pkg` | `uptime`, `vm_stat`, `df`, `pmset` 기반 sampler, future IOKit/sysctl |
| Windows | Windows Service | signed `.msi` | planned PDH/WMI/CIM/ETW/Event Log adapter |

개발 step:

1. MVP: Linux와 macOS sampler를 같은 pipeline으로 내보내고 source attribute로 구분한다.
2. 다음 단계: Windows sampler와 source 계약을 추가하고 Windows runner에서 service smoke test를 돌린다.
3. Production: package signing, install/uninstall, rollback, service restart, `doctor` 또는 `--check`
   명령을 OS별로 제공한다.

## Use Case 1: 단일 서버 상태를 바로 관측한다

제품 경험: 운영자가 서버 한 대에 client와 agent를 띄우면 CPU, memory, filesystem, network 같은
host/system 신호가 즉시 console에 보인다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-fe
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-agent
```

개발 step:

1. MVP: 현재 15초 cycle과 system metric sampler를 안정화한다.
2. 다음 단계: metric source별 count와 last send result를 agent log에 구조화한다.
3. Production: client 미연결 시 buffer/drop policy를 명시하고 health endpoint를 추가한다.

## Use Case 2: 현장 Gateway가 여러 Capability Node를 받는다

제품 경험: 한 site gateway가 edge 장비, file node, compute advisor의 metric을 받아 중앙 client로
보낸다. 사용자는 site 단위로 "이 gateway 주변에서 무슨 일이 나는지"를 본다.

개발 step:

1. MVP: `SKID_MONITOR_DEVICE_LISTEN_ADDR=127.0.0.1:9101`에서 local node push를 받는다.
2. 다음 단계: trusted LAN/overlay IP bind를 문서화하고 peer/source별 수신 count를 기록한다.
3. Production: SKDM v1, overlay VPN/service mesh 기반 전송 보호, authentication, read timeout,
   connection cap, rate limit을 넣는다.

## Use Case 3: Same-Pod Sidecar로 Workload 옆 신호를 모은다

제품 경험: Kubernetes Pod 안에서 app 옆에 agent와 file/compute node를 붙여, Pod 밖에 device socket을
열지 않고 workload-local capability를 관측한다.

개발 step:

1. MVP: loopback device ingress와 local node 조합을 예제로 고정한다.
2. 다음 단계: readiness/liveness check와 graceful shutdown을 추가한다.
3. Production: NetworkPolicy, restricted security context, read-only volume 예시 manifest를 제공한다.

## Use Case 4: Client로 전달되는 관측 Stream의 Gateway가 된다

제품 경험: 사용자는 agent 자체 수집 신호와 device ingress 신호를 구분하지 않고 하나의 client에서
본다. agent는 제품 데이터의 forwarder다.

개발 step:

1. MVP: 받은 `Signal`을 그대로 `transport::send`로 forward한다.
2. 다음 단계: blocking TCP send를 tokio I/O 또는 `spawn_blocking`으로 격리한다.
3. Production: client-subscribe 모델 또는 multi-client fan-out을 별도 transport로 설계한다.

## Use Case 5: Database 로그 파일을 OTLP Logs로 보낸다

제품 경험: 운영자가 agent에 PostgreSQL, MySQL, Redis, Valkey 등의 로그 파일을 등록하면 새로 추가된
로그 줄이 기존 logs pipeline을 통해 client 또는 외부 OTLP collector로 전달된다. 별도 DB 전송
프로토콜은 사용하지 않으며 각 레코드에는 `db.system.name`, `db.namespace`, `log.file.path`가 붙는다.

```json
{
  "receivers": {
    "database_logs": {
      "enabled": true,
      "poll_interval_millis": 1000,
      "start_at": "end",
      "max_line_bytes": 65536,
      "max_read_bytes": 1048576,
      "sources": [
        {
          "system": "postgresql",
          "path": "/var/log/postgresql/postgresql.log",
          "namespace": "orders",
          "service_name": "orders-postgresql",
          "instance": "primary"
        },
        {
          "system": "mysql",
          "path": "/var/log/mysql/error.log"
        },
        {
          "system": "redis",
          "path": "/var/log/redis/redis-server.log"
        },
        {
          "system": "valkey",
          "path": "/var/log/valkey/valkey.log"
        }
      ]
    }
  },
  "pipelines": {
    "logs": {
      "receivers": ["self_observation", "device", "otlp", "database_logs"],
      "exporters": ["skid"]
    }
  }
}
```

`start_at`의 기본값은 `end`다. agent 시작 전에 쌓인 전체 파일을 backfill하려면 `beginning`을
사용한다. offset은 현재 process 안에서 유지되며 truncate와 일반적인 파일 교체 rotation을 감지한다.
DB 설정에서 password나 bind parameter를 로그에 남기지 않도록 redaction 정책을 먼저 적용해야 한다.

개발 step:

1. MVP: 설정 파일 tail, rotation/truncate, 부분 줄, 크기 제한과 OTLP Logs 변환을 제공한다.
2. 다음 단계: DB별 multiline parser와 원본 timestamp parser를 선택적으로 추가한다.
3. Production: restart-safe offset checkpoint, backpressure/drop metric, secret redaction processor를 추가한다.
