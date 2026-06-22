//! monitor-cat client.
//!
//! server agent가 보낸 신호를 수신하여 사용자에게 모니터링 정보를 보여주는 곳.

mod receiver;
mod view;

fn main() {
    println!("monitor-cat client starting...");
}
