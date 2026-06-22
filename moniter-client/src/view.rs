//! 표시 계층.
//!
//! 수신한 신호를 사용자가 볼 수 있도록 렌더링한다.

use interface::protocol::Signal;

/// 수신한 신호를 사용자에게 보여준다.
pub fn render(signal: &Signal) {
    // TODO: 실제 UI/표시 구현
    match signal {
        Signal::Metrics(metrics) => println!("metrics: {} 건", metrics.len()),
        Signal::Alert { message } => println!("alert: {message}"),
    }
}
