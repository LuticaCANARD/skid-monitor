//! monitor-cat server agent.
//!
//! 호스트/인프라에서 정보를 수집(OpenTelemetry, k8s 인프라 시스템 등)하여
//! `interface` 프로토콜로 client에 전송하는 수집 agent.

mod collector;
mod transport;

fn main() {
    println!("monitor-cat server agent starting...");
}
