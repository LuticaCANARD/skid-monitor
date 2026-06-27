# Tauri UI and VRM Player Direction

skid-monitor의 프론트엔드 방향으로 Tauri UI에 VRM 재생기를 넣는 것은 좋은 선택지다.
다만 메인 UI를 VRM에 의존시키기보다는, VRM을 관측 상태를 표현하는 캐릭터 레이어로
두는 편이 더 안정적이고 확장하기 쉽다.

## 판단

Tauri UI와 VRM 조합은 skid-monitor의 성격과 잘 맞는다. 단순한 모니터링 대시보드보다
기억에 남는 경험을 만들 수 있고, `skid-monitor`라는 이름에도 자연스럽게 연결된다.

권장 방향은 다음과 같다.

- 대시보드는 실용적이고 읽기 쉬운 관측 화면으로 유지한다.
- VRM은 metric, log, trace, alert 상태를 직관적으로 증폭하는 표현 계층으로 둔다.
- 초기 구현에서는 "VRM 재생기" 자체보다 `Signal -> 캐릭터 상태` 매핑을 작게 잡는다.

## Architecture Sketch

```text
skid-monitor-agent / skid-edge-agent
        |
        v
skid-monitor-client or Tauri backend
        |
        v
Tauri event: signal_received
        |
        v
Frontend store
   |             |
Dashboard     VRM state machine
```

Rust 쪽은 기존 수신 로직을 재사용하고, Tauri backend가 받은 `Signal`을 frontend event로
emit한다. Frontend는 대시보드 상태와 VRM 상태 머신을 분리해서 관리한다.

## State Mapping Ideas

| Signal condition | VRM expression |
| --- | --- |
| 정상 상태 | idle, 가벼운 breathing |
| latency 증가 | 찡그린 표정, 느린 움직임 |
| error log 폭증 | 당황한 표정, 빠른 시선 이동 |
| edge device 온도 상승 | 땀, 붉은 조명, 더운 제스처 |
| alert 발생 | 화면 앞으로 다가오기, 알림 제스처 |

## Technical Notes

Tauri frontend에서는 Three.js 기반 VRM 로더를 사용하는 구성이 무난하다. Rust backend는
신호 수신과 프로토콜 처리를 맡고, WebView 안의 frontend가 렌더링과 캐릭터 상태 전환을
담당한다.

확인해야 할 리스크는 다음과 같다.

- WebView의 WebGL 호환성
- GPU 사용량과 배터리 영향
- VRM 모델 파일 크기
- Linux 환경의 WebView 렌더링 차이
- 대시보드 가독성과 캐릭터 연출 사이의 균형

## MVP Scope

초기 버전은 다음 범위로 시작한다.

1. Tauri shell을 만들고 기존 client 수신 흐름을 backend로 옮긴다.
2. `Signal`을 frontend event로 전달한다.
3. frontend store에서 최근 signal과 요약 상태를 관리한다.
4. VRM scene은 idle, warning, alert 정도의 작은 상태만 지원한다.
5. 대시보드는 VRM과 독립적으로 기본 metrics/logs/traces를 표시한다.

이 범위까지 구현하면, VRM이 단순 장식인지 실제 관측 경험을 돕는지 빠르게 검증할 수 있다.
