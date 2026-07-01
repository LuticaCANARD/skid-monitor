# RFCs

Skid Monitor의 구조적 결정과 운영 계약을 기록한다.

## Umbrella Rule

`skid-monitor`는 SKID 계열 실험을 합치는 canonical integration repository다. 다른 `skid-*`
repository는 조사와 설계의 source로만 취급하고, 채택할 기능은 먼저 RFC에 기록한다. 실제 구현은 이
repository 안에서만 진행한다.

## Index

Status 범례: Draft = 설계 합의 중인 forward-looking 문서로 코드를 권위로 삼지 않으며 구현 정합
검증 대상이 아니다. Accepted = 설계가 고정된 문서. Superseded = 후속 RFC로 대체된 문서.

| RFC | Status | Title | Scope |
| --- | --- | --- | --- |
| [0001](0001-initial-skid-monitor-integration.md) | Draft | Initial Skid Monitor Integration | 배포, 설정, device frame, compute probe, stream telemetry |
| [0002](0002-extensible-media-provider.md) | Draft | Extensible Edge Media Provider Contract | camera/image/video provider, edge adapter, preview boundary |

## Notes

초기 통합 설계는 RFC 0001을 정준 출처로 삼는다. source/as_str/node kind 매핑, 환경변수, metric
명명 규칙, framing 상한도 모두 RFC 0001 안의 Canonical Terms 절에서 관리한다. media provider처럼
특정 영역의 계약이 커지는 경우에는 후속 RFC에서 확장 계약을 분리한다.
