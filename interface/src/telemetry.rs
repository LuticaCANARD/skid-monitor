//! 메트릭 외 텔레메트리 신호(트레이스/로그)의 전송용 표현.
//!
//! server가 OpenTelemetry SDK에서 수집한 span/log를 client로 보내기 위한 직렬화 가능한 타입들이다.
//! OTel SDK 타입을 그대로 노출하지 않고, 경계를 넘기기 좋은 평탄한 형태로 변환해 담는다.

use serde::{Deserialize, Serialize};

/// 완료된 트레이스 span 하나.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    /// span 이름.
    pub name: String,
    /// 16진 trace id.
    pub trace_id: String,
    /// 16진 span id.
    pub span_id: String,
    /// span 지속 시간(밀리초).
    pub duration_ms: f64,
    /// span attribute를 평탄화한 키-값 목록.
    pub attributes: Vec<(String, String)>,
}

/// 로그 레코드 하나.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// 심각도 텍스트 (예: "INFO", "ERROR").
    pub severity: String,
    /// 로그 본문.
    pub body: String,
    /// 로그 attribute를 평탄화한 키-값 목록.
    pub attributes: Vec<(String, String)>,
}
