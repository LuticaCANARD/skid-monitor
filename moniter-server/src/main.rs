//! monitor-cat server agent.
//!
//! 호스트/인프라에서 정보를 수집(OpenTelemetry, k8s 인프라 시스템 등)하여
//! `interface` 프로토콜로 client에 전송하는 수집 agent.
//!
//! 현재는 server 자신을 OpenTelemetry SDK로 자체 계측하여 metrics/traces/logs를 생성하고,
//! 이를 `interface` 신호로 변환해 [`transport`]로 내보낸다.

mod collector;
mod telemetry;
mod transport;

use interface::protocol::Signal;
use std::time::Duration;
use tracing::{info, instrument};

/// 수집 주기.
const CYCLE_INTERVAL: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() {
    let guard = telemetry::init();
    info!("monitor-cat server agent starting...");

    let mut interval = tokio::time::interval(CYCLE_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => run_cycle(&guard).await,
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received");
                break;
            }
        }
    }

    // 배치 워커에 남은 신호를 flush 한다.
    guard.shutdown();
}

/// 한 주기: 자체 계측한 세 신호를 읽어 client로 전송한다.
///
/// `#[instrument]`로 이 함수 자체가 OTel span이 되고, 그 span은 다음 주기의 trace 수집에 잡힌다
/// (self-observation).
#[instrument(skip(guard))]
async fn run_cycle(guard: &telemetry::TelemetryGuard) {
    let metrics = collector::collect(guard);
    info!(count = metrics.len(), "collected metrics");
    transport::send(Signal::Metrics(metrics));

    let spans = collector::collect_spans(guard);
    transport::send(Signal::Traces(spans));

    let logs = collector::collect_logs(guard);
    transport::send(Signal::Logs(logs));
}
