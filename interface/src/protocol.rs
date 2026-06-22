//! server(agent)와 client가 주고받는 메시지 정의.
//!
//! server는 [`Signal`]을 보내고, client는 이를 수신해 사용자에게 보여준다.

use crate::metrics::Metric;

/// server agent가 client로 전송하는 신호.
#[derive(Debug, Clone)]
pub enum Signal {
    /// 주기적으로 수집된 지표 묶음.
    Metrics(Vec<Metric>),
    /// 임계치 초과 등 즉시 알려야 하는 경보.
    Alert { message: String },
}
