# RFCs

Skid Monitor의 구조적 결정과 운영 계약을 기록한다.

## Umbrella Rule

`skid-monitor`는 SKID 계열 실험을 합치는 canonical integration repository다. 다른 `skid-*`
repository는 조사와 설계의 source로만 취급하고, 채택할 기능은 먼저 RFC에 기록한다. 실제 구현은 이
repository 안에서만 진행한다.

## Status 기준

| Status | 의미 |
| --- | --- |
| Draft | 의견 수렴 중이며 구현 기준이 아니다. 이미 존재하는 code는 별도 implementation checklist로 표시한다. |
| Accepted | 구현이 따라야 하는 결정으로 채택됐다. 구현 완료를 뜻하지 않는다. |
| Implemented | 채택된 결정의 필수 범위가 code와 test에 반영됐다. |
| Superseded | 다른 RFC로 대체됐다. 대체 RFC를 명시해야 한다. |
| Rejected | 검토했지만 채택하지 않았다. |

RFC status와 제품 기능 상태는 다르다. 제품의 Stable/Experimental/Prototype/Planned 상태는
[Feature Status](../feature-status.md)가 정준 출처다.

## Index

| RFC | Status | Title | Scope |
| --- | --- | --- | --- |
| [0001](0001-initial-skid-monitor-integration.md) | Draft | Initial Skid Monitor Integration | 배포, 설정, device frame, compute probe, stream telemetry |
| [0002](0002-extensible-media-provider.md) | Draft | Extensible Edge Media Provider Contract | camera/image/video provider, edge adapter, preview boundary |

## Notes

초기 통합 설계는 RFC 0001을 정준 출처로 삼는다. source/as_str/node kind 매핑, 환경변수, metric
명명 규칙, framing 상한도 모두 RFC 0001 안의 Canonical Terms 절에서 관리한다. media provider처럼
특정 영역의 계약이 커지는 경우에는 후속 RFC에서 확장 계약을 분리한다.
