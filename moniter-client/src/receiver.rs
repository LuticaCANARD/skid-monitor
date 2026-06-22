//! 수신 계층.
//!
//! server agent가 보낸 [`interface::protocol::Signal`]을 받아온다.

use interface::protocol::Signal;

/// 다음 신호를 수신한다. (없으면 `None`)
pub fn recv() -> Option<Signal> {
    // TODO: server로부터의 신호 수신(네트워크) 구현
    None
}
