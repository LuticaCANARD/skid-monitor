# skid-protocol Use Cases

## 1. Binary Crate 간 Signal 공유

agent, client, edge/file/compute node는 모두 `skid_protocol::protocol::Signal`을 사용한다. 한쪽에서
JSON으로 직렬화한 `Signal`을 다른 쪽에서 같은 타입으로 decode한다.

## 2. 간단 Metric을 OTLP Request로 변환

센서 값이나 filesystem snapshot처럼 SDK aggregator를 거치지 않는 값은 `Metric` 목록으로 만든 뒤
`export_metrics`로 OTLP `ExportMetricsServiceRequest`로 감싼다.

## 3. Source별 Resource Attribute 일관화

`Source::as_str()`는 `skid_monitor.source` resource attribute의 정준 값을 제공한다. node별 metric은
이 값을 통해 client와 extension에서 source를 구분한다.

## 4. 테스트용 Signal Fixture 작성

client receiver, view formatter, transport 코드는 `skid-protocol`의 타입으로 fixture를 만들 수 있다.
이 덕분에 binary별 테스트가 wire schema를 따로 복제하지 않는다.
