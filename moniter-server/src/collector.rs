//! 정보 수집 계층.
//!
//! OpenTelemetry, 쿠버네티스 인프라/시스템 등에서 지표를 수집해
//! [`interface::metrics::Metric`]으로 변환한다.

use interface::metrics::Metric;

/// 한 주기 동안 수집한 지표를 반환한다.
pub fn collect() -> Vec<Metric> {
    // TODO: OpenTelemetry / k8s 수집 구현
    Vec::new()
}
