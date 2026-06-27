# skid-protocol Product Use Cases

`skid-protocol`은 사용자가 직접 실행하는 제품 화면은 아니지만, 모든 Skid Monitor 제품 경험을
같은 언어로 묶는 계약 계층이다. 아래 use case는 "제품에서 어떤 경험을 만들기 위해 이 crate가
필요한가"를 기준으로 정리한다.

## Kubernetes 운용 호환성

Kubernetes에서 protocol crate는 runtime에 묶이면 안 된다. Pod, namespace, node, container 같은
Kubernetes 식별자는 metric attribute로 실을 수 있어야 하지만, `skid-protocol` 자체가 Kubernetes API나
tokio runtime을 의존해서는 안 된다.

개발 step:

1. MVP: Kubernetes 관련 값도 일반 attribute로 표현하고 protocol 타입은 그대로 유지한다.
2. 다음 단계: `k8s.namespace.name`, `k8s.pod.name`, `k8s.node.name` 같은 attribute 이름을 fixture로
   검증한다.
3. Production: rolling upgrade 중 구버전 Pod와 신버전 Pod가 같은 `Signal`을 읽을 수 있도록 envelope
   version과 unknown field policy를 고정한다.

## Use Case 1: 모든 Node가 같은 Signal을 말한다

제품 경험: 운영자는 edge sensor, file node, compute advisor, host collector에서 온 신호를 하나의
client 화면에서 본다. source가 달라도 payload는 `Signal`로 통일되어 agent와 client가 같은 decode
경로를 쓴다.

개발 step:

1. MVP: `Signal::Metrics`, `Signal::Traces`, `Signal::Logs`를 유지하고 binary crate가 이 타입만
   직렬화하도록 한다.
2. 다음 단계: source별 fixture를 추가해 client render와 extension host가 같은 schema를 소비하는지
   검증한다.
3. Production: protocol version과 node identity envelope를 추가해 rolling upgrade와 unknown field
   처리를 명확히 한다.

## Use Case 2: 제품 화면에서 Source를 일관되게 필터링한다

제품 경험: client나 extension은 `skid_monitor.source=edge_device` 같은 resource attribute를 기준으로
"장비 신호", "파일 capability", "compute advisor"를 구분한다. 사용자는 source별 탭, 필터, 요약을
기대한다.

개발 step:

1. MVP: `Source::as_str()` 값을 정준으로 삼고 모든 metric export가 이 값을 resource attribute에
   싣게 한다.
2. 다음 단계: `Source::Stream` 등 future source를 추가할 때 client fallback display를 함께 만든다.
3. Production: source 값과 node kind 값의 호환성 테스트를 두어 schema drift를 막는다.

## Use Case 3: 간단 Metric을 OTLP로 감싸 제품 데이터로 만든다

제품 경험: edge 온도, file root size, logical CPU count 같은 단일 표본도 OpenTelemetry 기반 화면과
extension에서 같은 방식으로 읽힌다.

개발 step:

1. MVP: `Metric` 목록을 `export_metrics`로 OTLP metrics request에 넣는다.
2. 다음 단계: metric name, unit, `MetricKind` 검증 helper를 추가해 잘못된 표본을 줄인다.
3. Production: metric schema snapshot 테스트를 만들어 UI와 extension이 의존하는 이름을 보호한다.

## Use Case 4: 통합 테스트용 제품 Fixture를 만든다

제품 경험: 새 node나 client view를 만들 때 실제 agent를 모두 띄우지 않아도 제품에 가까운 signal을
재현할 수 있다.

개발 step:

1. MVP: crate별 테스트에서 `skid-protocol` 타입으로 fixture를 직접 만든다.
2. 다음 단계: edge/file/compute 대표 fixture builder를 protocol test module에 둔다.
3. Production: fixture를 golden JSON으로 보관해 wire compatibility regression을 잡는다.
