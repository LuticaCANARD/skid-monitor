# Edge Agent Deployment

이 문서는 `moniter-edge-agent`와 `moniter-server`의 관계, 배포 경계, 설치 방식의
기본 결정을 명시한다.

## Role

`moniter-edge-agent`는 현장 장비 가까이에서 물리 계층과 환경 신호를 만드는 probe다.
`moniter-server`는 중앙 또는 사이트별 수집 지점으로, edge agent가 보낸 `Signal`을 받아
자체 host/system metrics와 함께 `moniter-client`로 전달한다.

관계는 다음과 같다.

```text
moniter-edge-agent
  -> edge physical metrics 생성
  -> length-prefixed JSON Signal 전송
  -> MONITOR_CAT_DEVICE_ADDR 로 접속

moniter-server
  -> MONITOR_CAT_DEVICE_LISTEN_ADDR 에서 장비 소켓 listen
  -> edge agent가 보낸 Signal 수신
  -> 자체 system metrics와 함께 client로 전달

moniter-client
  -> 사람이 보는 화면
```

따라서 `moniter-edge-agent`는 `moniter-server` 안에 포함되는 하위 모듈이 아니다. 같은
workspace와 `interface` 계약을 공유하지만, 실행과 배포 단위는 별도 바이너리로 둔다.

## Network Model

현재 모델은 edge agent가 server로 신호를 push하는 방식이다. server가 edge agent에 접속해
pull하지 않는다.

```sh
# server (보안 기능이 들어가기 전에는 loopback 또는 신뢰된 내부 IP에만 bind)
MONITOR_CAT_DEVICE_LISTEN_ADDR=127.0.0.1:9101 moniter-server

# edge node 또는 gateway
MONITOR_CAT_DEVICE_ADDR=server-ip-or-hostname:9101 moniter-edge-agent
```

> `0.0.0.0:9101`로 모든 인터페이스에 bind하는 것은 device 인증이 들어간 뒤로 미룬다.
> 현재 소켓은 인증이 없고 받은 `Signal`을 검증 없이 client로 forward하므로, 공개 bind는
> 임의 신호 주입과 메모리 고갈 DoS의 직접 경로가 된다. 아래 Security Requirements 참고.

### push 모델의 신뢰성 한계

push를 택했지만 현재 edge agent에는 **재시도, 백오프, 로컬 버퍼링이 없다.** 전송 실패 시
stderr에 기록하고 다음 주기로 넘어가며, 그 사이 신호는 영구 손실된다. server 재시작이나
네트워크 단절 구간의 brownout, watchdog reset 같은 장애 신호가 정확히 그 순간 유실되기
가장 쉽다. 운영 전에는 최소한 재연결 + 백오프, 가능하면 짧은 로컬 버퍼(전송 실패분 보관)를
추가한다. pull 또는 hybrid 모델은 server 복구 시 재수집 여지를 주므로 장기적으로 재검토 대상이다.

하나의 `moniter-server`에는 여러 `moniter-edge-agent`가 연결될 수 있다.

현재 구현에서 각 agent가 metric에 붙이는 attribute는 `device_id`, `node_name`,
`sensor` 세 가지다. `rack`, `zone` 같은 위치 attribute는 아직 코드에 없으며, 도입하려면
설정(env)과 attribute 채움 양쪽을 함께 추가해야 한다. 즉 이 둘은 현재 구현이 아니라
향후 식별 모델의 목표값이다.

### Connection lifecycle (현재 구현의 한계)

현재 `moniter-server`의 장비 소켓은 **연결당 단일 `Signal` 프레임을 읽고 연결을 닫는다.**
그래서 edge agent도 매 전송 주기마다 `connect -> 1 frame -> close`를 반복한다. 상시 유지되는
세션이 아니다. 이 모델은 다음 한계를 동반한다.

- 연결 자체로는 heartbeat/last-seen을 알 수 없다. 신호가 끊긴 것인지 단지 다음 interval이
  안 된 것인지 server가 연결 수준에서 구분하지 못한다. 따라서 아래 Security Requirements의
  device heartbeat는 별도 애플리케이션 레벨 상태 테이블과 타임아웃으로 구현해야 한다.
- 매 주기마다 TCP handshake를 반복한다. mTLS를 얹으면 매 전송마다 TLS handshake까지
  반복되어 비용이 커진다. 장기 실행 probe라는 성격과 단명 연결은 어긋난다.

운영 전 개선 목표는 영속 연결 위에서 프레임을 스트리밍하고, 그 연결에 heartbeat를 실어
last-seen을 갱신하는 형태다.

## Deployment Decision

기본 배포 방식은 단일 Rust 바이너리와 systemd 서비스다.

이 결정을 기본값으로 두는 이유는 다음과 같다.

- edge agent는 장비 또는 gateway에서 계속 떠 있어야 하는 장기 실행 프로세스다.
- GPIO, I2C, serial, 온도 센서, 전원 상태 같은 로컬 하드웨어를 직접 읽을 가능성이 높다.
- 컨테이너는 `/dev` 마운트, privileged 권한, host network 같은 설정이 늘어나기 쉽다.
- 작은 단일 바이너리는 현장 장비에 설치, 교체, 롤백하기 쉽다.

컨테이너 이미지는 센서 없는 lab 환경이나 gateway 시뮬레이션에는 사용할 수 있지만, 기본
운영 배포 모델로 보지는 않는다.

단, 하드웨어 직접 접근을 근거로 들었으면 그에 따르는 권한 모델도 배포 단위의 일부로 둔다.
systemd unit은 root 상시 실행 대신 전용 사용자 + 필요한 device만 `DeviceAllow`로 허용하고,
serial/GPIO는 `dialout`/`gpio` 그룹 또는 udev 규칙으로 부여하는 것을 기본 형태로 한다.
이 권한 설계 없이 단순 root 실행으로 두면 "컨테이너 권한이 늘기 쉽다"는 회피 논거가 약해진다.

## Recommended Install Shape

초기 설치 흐름은 다음을 목표로 한다.

1. GitHub Release 또는 내부 release 저장소에 target별 바이너리를 게시한다.
2. 설치 스크립트가 바이너리를 `/usr/local/bin/moniter-edge-agent`에 배치한다.
3. 설정 파일을 `/etc/monitor-cat/edge-agent.env`에 만든다.
4. systemd unit을 `/etc/systemd/system/moniter-edge-agent.service`에 만든다.
5. `systemctl enable --now moniter-edge-agent`로 부팅 시 자동 실행되게 한다.

설정 파일 예시는 다음과 같다.

```sh
MONITOR_CAT_DEVICE_ADDR=10.0.0.5:9101
MONITOR_CAT_EDGE_DEVICE_ID=edge-dev-001
MONITOR_CAT_EDGE_NODE=factory-a-line-3
MONITOR_CAT_EDGE_INTERVAL_SECS=15
```

사용자가 기대하는 설치 UX는 다음 형태다.

```sh
curl -fsSL https://example.com/monitor-cat/moniter-edge-agent/install.sh | sudo sh -s -- \
  --server 10.0.0.5:9101 \
  --device-id edge-dev-001 \
  --node factory-a-line-3
```

`curl | sudo sh`는 시연용 UX일 뿐, 운영 기본 경로로 보지 않는다. 이 방식만으로는 바이너리
무결성 검증(서명/체크섬), 멱등 재설치, 깔끔한 언인스톨/롤백을 보장하지 못한다. 롤백이 쉽다는
단일 바이너리의 장점을 실제로 살리려면 아래 Release Targets의 `.deb`/`.rpm` + 내부 저장소를
"확장"이 아니라 권장 경로로 앞당기고, install.sh에는 최소한 체크섬 검증 단계를 포함한다.

## Release Targets

초기 release에는 다음 Linux target을 우선한다.

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `armv7-unknown-linux-gnueabihf`

운영 환경이 다양해지면 `musl` 기반 정적 바이너리도 제공한다. 장비 수가 늘어나면
`install.sh` 중심 배포에서 `.deb` 또는 `.rpm` 패키지와 내부 패키지 저장소로 확장한다.

## Versioning

`moniter-edge-agent`와 `moniter-server`는 독립 실행 파일이지만 같은 `interface` 계약을
공유한다. 초기에는 같은 repository release 안에서 같은 버전으로 묶어 배포한다.

현재 `interface`의 `Signal`은 버전 필드가 없는 plain enum이고, serde_json의
externally-tagged 표현으로 직렬화된다. 이 상태에서는 나중에 envelope를 끼워 버전을
넣는 것 자체가 breaking change다. 따라서 protocol version은 "프로토콜이 바뀌면 추가"하는
항목이 아니라, **호환 깨짐이 비싸지기 전인 지금 envelope에 먼저 넣어야** 하는 선행 작업이다.

도입 시 함께 정하는 것:

- `Signal` envelope의 protocol version 필드 (지금 추가)
- server의 backward-compatible decode 규칙 (알 수 없는 variant/필드 무시)
- agent와 server의 최소 호환 버전 명시

이와 별개로 인코딩 변경(CBOR/postcard 등)은 현재 edge agent가 Linux 위 Rust 바이너리이지
MCU 펌웨어가 아니라는 점을 전제로 판단한다. JSON 제약은 아직 MCU 제약이 아니므로, compact
인코딩은 실제로 펌웨어에 직접 올리는 probe가 생기거나 대역폭 제약이 측정될 때 도입한다.

## Security Requirements

현재 구현은 plain TCP와 JSON frame을 사용하고, 장비 소켓에는 인증이 없다. 또한 server는 받은
`Signal`을 검증 없이 그대로 client로 forward한다. 따라서 소켓에 접근 가능한 누구든

- 가짜 metric이나 임의 `Signal::Alert`를 주입해 client 화면을 오염시킬 수 있고,
- 16 MiB 프레임 상한과 연결 수 제한 없는 수락(connection당 unbounded task spawn)을 이용해
  메모리 고갈 DoS를 일으킬 수 있다.

production 배포 전에는 다음 항목을 추가해야 한다.

- TLS, mTLS, WireGuard, Tailscale 중 하나를 통한 전송 보호
- device enrollment token 또는 shared secret 기반 장비 인증
- 동시 연결 수 제한과 per-connection backpressure (현재는 무제한 spawn)
- server의 device heartbeat와 last-seen 상태 관리 (단명 연결 구조상 애플리케이션 레벨로 구현)
- agent 로그와 restart 상태 확인 방법

보안 기능이 들어가기 전에는 `0.0.0.0` 공개 bind를 피하고, 같은 trusted LAN 또는 개발
환경에서 loopback/신뢰된 내부 IP에만 bind해 사용한다.

## Summary

`moniter-server`는 edge 신호를 받아 client로 전달하는 gateway이고, `moniter-edge-agent`는
현장 장비 가까이에서 물리 신호를 만들어 보내는 probe다. 배포 기본값은 정적 또는 단일
바이너리, env 설정 파일, systemd 서비스다.
