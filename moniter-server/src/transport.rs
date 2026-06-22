//! 전송 계층.
//!
//! 수집한 정보를 [`interface::protocol::Signal`]에 담아 client로 보낸다.

use interface::protocol::Signal;

/// 신호를 client로 전송한다.
pub fn send(_signal: Signal) {
    // TODO: client로의 전송(네트워크) 구현
}
