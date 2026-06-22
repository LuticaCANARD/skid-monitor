//! 수집되는 모니터링 데이터의 타입 정의.
//!
//! server agent가 OpenTelemetry / k8s 인프라 등에서 수집한 값을 이 타입들로 표현한다.
//! 직렬화가 필요해지면 serde derive를 여기에 추가한다.

/// 단일 측정 표본.
#[derive(Debug, Clone)]
pub struct Metric {
    /// 측정 이름 (예: "cpu.usage", "pod.restart_count").
    pub name: String,
    /// 측정 값.
    pub value: f64,
    /// 측정이 발생한 출처.
    pub source: Source,
}

/// 측정이 발생한 출처 구분.
#[derive(Debug, Clone)]
pub enum Source {
    /// OpenTelemetry 계측에서 온 값.
    OpenTelemetry,
    /// 쿠버네티스 인프라/시스템에서 온 값.
    Kubernetes,
    /// 그 외 호스트 시스템 지표.
    System,
}
