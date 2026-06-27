# skid-monitor-client Product Use Cases

`skid-monitor-client`는 사용자가 직접 만지는 제품 표면이다. 목표는 signal dump가 아니라, Skid Monitor
신호를 사람이 판단 가능한 상태 요약과 extension event로 바꾸는 것이다.

## Kubernetes 운용 호환성

client는 운영자 workstation에서 실행해 port-forward나 overlay 주소로 agent를 보는 형태와, cluster
안에서 internal dashboard sink로 실행하는 형태를 모두 고려한다. public Service로 바로 노출하지 않고,
접근 제어와 preview 권한은 extension/GUI layer에서 명시적으로 다룬다.

개발 step:

1. MVP: local client + `kubectl port-forward` 또는 overlay 주소로 agent/client transport를 검증한다.
2. 다음 단계: in-cluster 실행 시 ConfigMap/Secret 기반 extension 설정과 read-only filesystem을
   지원한다.
3. Production: dashboard 배포에는 authentication, RBAC-aware access path, NetworkPolicy, extension
   permission manifest를 함께 둔다.

## Use Case 1: 운영자가 Terminal에서 전체 상태를 본다

제품 경험: 운영자는 agent 주소를 맞춘 뒤 client를 실행하고, metrics/traces/logs 요약을 terminal에서
바로 본다. 개발자 도구처럼 시작하지만 제품의 첫 화면 역할을 한다.

```sh
SKID_MONITOR_CLIENT_ADDR=127.0.0.1:9000 cargo run -p skid-monitor-client
```

개발 step:

1. MVP: 현재 count와 data point text render를 안정화한다.
2. 다음 단계: source별 section, severity hint, last-seen timestamp를 추가한다.
3. Production: terminal view를 "site summary", "node detail", "recent events"처럼 탐색 가능한 구조로
   나눈다.

## Use Case 2: 현장 신호를 Extension으로 제품화한다

제품 경험: 사용자는 Rust client를 유지하면서 C# extension으로 avatar state, Unity bridge, custom
alert classifier 같은 제품 기능을 붙인다. extension이 실패해도 기본 monitor는 살아 있다.

개발 step:

1. MVP: `SKID_MONITOR_EXTENSION_HOST`와 `SKID_MONITOR_DOTNET_EXTENSIONS`로 extension host를 실행한다.
2. 다음 단계: extension event에 source, timestamp, client version을 명시한다.
3. Production: extension manifest, permission, timeout, restart policy를 추가한다.

## Use Case 3: Demo Booth / Lab Dashboard

제품 경험: lab machine 한 대에서 client가 edge temperature, file root size, compute capability를 한
화면에 보여준다. 사용자에게 "Skid Monitor가 무엇을 모으는지"를 빠르게 설명하는 제품 demo다.

개발 step:

1. MVP: agent + edge/file/compute `--once` 흐름을 README 예제로 묶는다.
2. 다음 단계: sample data를 source별로 보기 좋게 render한다.
3. Production: scripted demo mode와 replay fixture를 제공해 live node 없이도 제품 tour를 보여준다.

## Use Case 4: Stream Preview의 안전한 관문

제품 경험: stream telemetry가 들어오면 client는 up/down, fps, bitrate, endpoint 존재 여부를 보여준다.
실제 preview는 사용자가 명시적으로 켠 extension이나 GUI layer가 연다.

개발 step:

1. MVP: `skid_monitor.source=stream` metric을 unknown source로 깨지지 않게 표시한다.
2. 다음 단계: stream summary renderer와 endpoint redaction display를 추가한다.
3. Production: preview permission prompt와 audit log를 extension boundary에 붙인다.

## Use Case 5: File Offer를 보고 필요한 파일을 내려받는다

제품 경험: 사용자는 client에서 file node가 노출한 read-only offer를 보고 로그나 support artifact를
선택해 내려받는다. client는 요청을 시작하지만 실제 권한 확인과 chunk 제공은 agent/server와
`skid-file-node`가 담당한다.

개발 step:

1. MVP: file offer metadata를 source별 section에 표시한다.
2. 다음 단계: download request command를 추가하고 progress, bytes, 실패 사유를 보여준다.
3. Production: per-user permission, redaction warning, audit id, resumed download 상태를 UI에 노출한다.
