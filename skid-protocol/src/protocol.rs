//! server(agent)와 client가 주고받는 메시지 정의.
//!
//! server는 [`Signal`]을 보내고, client는 이를 수신해 사용자에게 보여준다.
//! server/client는 TCP 경계로 나뉠 수 있으므로 [`Signal`]은 serde로 직렬화 가능하다.
//! 각 텔레메트리 payload는 OTLP `Export*ServiceRequest` protobuf 모델을 그대로 사용한다.

use crate::otlp::{
    ExportLogsServiceRequest, ExportMetricsServiceRequest, ExportTraceServiceRequest,
};
use serde::{Deserialize, Serialize};

/// server agent가 client로 전송하는 신호.
#[derive(Clone, Serialize, Deserialize)]
pub enum Signal {
    /// OTLP metrics export request.
    Metrics(ExportMetricsServiceRequest),
    /// OTLP traces export request.
    Traces(ExportTraceServiceRequest),
    /// OTLP logs export request.
    Logs(ExportLogsServiceRequest),
}
