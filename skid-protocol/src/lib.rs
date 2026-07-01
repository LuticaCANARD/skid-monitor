//! # skid-protocol
//!
//! skid-monitor의 agent와 client가 공유하는 계약(contract) 라이브러리.
//!
//! - server는 [`metrics`]에 정의된 타입으로 수집한 정보를 채우고,
//!   [`protocol`]의 메시지에 담아 client로 전송한다.
//! - client는 동일한 [`protocol`] 메시지를 수신해 정보를 해석·표시한다.

pub mod frame;
pub mod metrics;
pub mod otlp;
pub mod protocol;
