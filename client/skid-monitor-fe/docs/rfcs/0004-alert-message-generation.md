# 알람 메시지 생성

## 상태

Draft

## 목표

알람 상태를 사용자가 바로 이해할 수 있는 짧은 문장으로 바꾼다.
메시지는 dashboard event log, toast, VRM 말풍선, 추후 desktop notification에서 재사용한다.

## 결정

MVP는 AI 생성 문장이 아니라 deterministic template 기반으로 시작한다.

이유:

1. 알람 문구는 정확성과 반복 가능성이 중요하다.
2. 같은 알람에는 같은 요약이 나와야 dedupe와 테스트가 쉽다.
3. offline 환경과 저사양 빌드에서도 동일하게 동작해야 한다.
4. 추후 AI 요약을 붙이더라도 template output을 안전한 입력으로 사용할 수 있다.

## 입력

메시지 생성기는 다음 값을 받는다.

- rule id
- severity
- status
- source
- metric name
- current value
- threshold
- 주요 attributes
- first fired time
- resolved time

## 출력

```rust
pub(crate) struct AlertMessage {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) short: String,
}
```

- `title`: event log와 notification title
- `body`: 자세한 dashboard 설명
- `short`: VRM 말풍선용 짧은 문장

## 문체

- 과장하지 않는다.
- 원인 단정은 피하고 관측된 사실을 말한다.
- 사용자가 다음 행동을 알 수 있게 한다.
- critical만 강한 표현을 사용한다.

예:

| severity | short |
| --- | --- |
| info | `새 상태 변화가 있어요.` |
| warning | `주의가 필요한 지표가 있어요.` |
| critical | `즉시 확인이 필요한 문제가 있어요.` |
| resolved | `문제가 해소됐어요.` |

## Template 초안

### CPU high

- title: `High CPU usage`
- body: `{source} reported {value}% CPU usage, above the {threshold}% warning threshold.`
- short: `CPU 사용률이 높아요.`

### Memory high

- title: `High memory usage`
- body: `{source} reported {value}% memory usage, above the {threshold}% warning threshold.`
- short: `메모리 사용률이 높아요.`

### File root unavailable

- title: `File root unavailable`
- body: `{root_label} at {root_path} is not accessible from {source}.`
- short: `파일 루트 접근이 안 돼요.`

### Receiver error

- title: `Signal receiver error`
- body: `The frontend receiver cannot accept monitor signals: {error}.`
- short: `신호 수신에 문제가 있어요.`

### Resolved

- title: `{alert_title} resolved`
- body: `{alert_title} is no longer firing for {source}.`
- short: `이 알람은 해소됐어요.`

## VRM 말풍선 제약

VRM presenter의 `short` 메시지는 한 줄에 가까워야 한다.

- 28자 안팎의 한국어 문장 권장
- metric value는 필요할 때만 넣는다.
- critical 상태에서는 가장 중요한 알람 하나만 말한다.
- 여러 알람이 있으면 `외 {n}건` 같은 축약을 presenter adapter에서 붙인다.

## 중복과 빈도 제어

메시지 생성기는 문장을 만들 뿐, 표시 빈도는 알람 상태 머신이 제어한다.

- `pending`: 기본적으로 메시지 없음
- `firing`: title/body/short 생성
- `acknowledged`: 짧은 확인 메시지
- `resolved`: 회복 메시지 1회

## 향후 AI 요약

AI 요약은 다음 조건을 만족할 때만 붙인다.

- template message가 이미 생성되어 있음
- 외부 전송 가능한 데이터 범위가 명확함
- network 실패 시 template message로 fallback 가능함
- 사용자가 끌 수 있음

AI 요약의 역할은 여러 알람을 사람이 읽기 쉽게 묶는 것이며, threshold 판정이나 severity 결정에는 관여하지 않는다.

## 수용 기준

- 같은 `AlertSnapshot`은 같은 `AlertMessage`를 만든다.
- unknown rule id도 generic fallback message를 만든다.
- VRM presenter용 `short`는 빈 문자열이 아니어야 한다.
- resolved 상태는 firing 상태와 다른 문구를 사용한다.
- 메시지 생성 테스트는 현재 시간에 의존하지 않는다.

## 열린 질문

- 한국어와 영어를 동시에 지원할지 결정해야 한다.
- 사용자 정의 rule의 template을 설정 파일에서 받을지 결정해야 한다.
- sound cue와 message severity를 같은 table에서 관리할지 결정해야 한다.
