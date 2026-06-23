//! 수집되는 모니터링 데이터의 타입 정의.
//!
//! server agent가 OpenTelemetry / k8s 인프라 등에서 수집한 값을 이 타입들로 표현한다.
//! server와 client는 TCP 경계로 나뉠 수 있으므로 모든 타입은 serde로 직렬화 가능하다.

use serde::{Deserialize, Serialize};

/// 단일 측정 표본.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// 측정 이름 (예: "cpu.usage", "pod.restart_count").
    pub name: String,
    /// 측정 값. 히스토그램 등 복합 집계는 대표값(sum 등)으로 평탄화한다.
    pub value: f64,
    /// 측정이 발생한 출처.
    pub source: Source,
    /// 측정 단위 (예: "ms", "By"). 없을 수 있다.
    pub unit: Option<String>,
    /// 집계 종류.
    pub kind: MetricKind,
    /// OpenTelemetry attribute 등 부가 속성을 평탄화한 키-값 목록.
    pub attributes: Vec<(String, String)>,
}

/// 측정이 발생한 출처 구분.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    /// OpenTelemetry 계측에서 온 값.
    OpenTelemetry,
    /// 쿠버네티스 인프라/시스템에서 온 값.
    Kubernetes,
    /// 그 외 호스트 시스템 지표.
    System,
}

/// 메트릭 집계 종류.
///
/// 히스토그램은 단일 `f64`로 표현하기 위해 대표값(sum)으로 평탄화하며, 버킷 정보는 손실된다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetricKind {
    /// 현재 값 측정.
    Gauge,
    /// 누적 합계.
    Sum,
    /// 분포 측정(대표값으로 평탄화됨).
    Histogram,
}
