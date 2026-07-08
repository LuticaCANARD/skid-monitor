//! OpenTelemetry 자체 계측 초기화/종료.
//!
//! server agent 자신을 OTel SDK로 계측해 metrics/traces/logs 세 신호를 생성한다.
//! 신호는 두 갈래로 흐른다.
//!
//! - **경로 1 (skid-monitor 파이프라인)**: in-memory exporter에 모아 두고, [`crate::collector`]가
//!   주기적으로 읽어 `skid_protocol` 타입으로 변환한 뒤 agent pipeline으로 보낸다.
//! - **경로 2 (옵션)**: 환경변수 `OTEL_EXPORTER_OTLP_ENDPOINT`가 설정돼 있으면 OTLP exporter를
//!   같은 provider에 병행 등록해 Jaeger/Collector 등 외부 백엔드로도 내보낸다.

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::logs::in_memory_exporter::InMemoryLogExporter;
use opentelemetry_sdk::metrics::in_memory_exporter::InMemoryMetricExporter;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::trace::in_memory_exporter::InMemorySpanExporter;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// 계측 서비스 이름. trace/metric/log의 `service.name` resource attribute로 쓰인다.
const SERVICE_NAME: &str = "skid-monitor-agent";

/// SDK provider와 in-memory exporter 핸들을 보관한다.
///
/// exporter 핸들은 provider 내부와 공유(Arc 기반 clone)되며, [`crate::collector`]가 이를 통해
/// 모인 신호를 읽는다. provider는 종료 시 [`Self::shutdown`]으로 flush 해야 한다.
pub struct TelemetryGuard {
    pub metric_exporter: InMemoryMetricExporter,
    pub span_exporter: InMemorySpanExporter,
    pub log_exporter: InMemoryLogExporter,
    pub resource: Resource,
    pub meter_provider: SdkMeterProvider,
    pub tracer_provider: SdkTracerProvider,
    pub logger_provider: SdkLoggerProvider,
}

/// OTel 파이프라인과 tracing subscriber를 초기화한다.
pub fn init() -> TelemetryGuard {
    let resource = Resource::builder().with_service_name(SERVICE_NAME).build();
    // 경로 2: OTLP endpoint가 지정돼 있을 때만 외부 export를 켠다.
    let otlp_on = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();

    // --- metrics: in-memory periodic reader (+ 옵션 OTLP reader) ---
    let metric_exporter = InMemoryMetricExporter::default();
    let mut meter_builder = SdkMeterProvider::builder()
        .with_resource(resource.clone())
        .with_reader(PeriodicReader::builder(metric_exporter.clone()).build());
    if otlp_on {
        let otlp_metrics = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()
            .expect("build OTLP metric exporter");
        meter_builder = meter_builder.with_reader(PeriodicReader::builder(otlp_metrics).build());
    }
    let meter_provider = meter_builder.build();
    global::set_meter_provider(meter_provider.clone());

    // --- traces: in-memory batch exporter (+ 옵션 OTLP batch exporter) ---
    let span_exporter = InMemorySpanExporter::default();
    let mut tracer_builder = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter.clone());
    if otlp_on {
        let otlp_spans = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .build()
            .expect("build OTLP span exporter");
        tracer_builder = tracer_builder.with_batch_exporter(otlp_spans);
    }
    let tracer_provider = tracer_builder.build();
    global::set_tracer_provider(tracer_provider.clone());
    let otel_span_layer =
        tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer(SERVICE_NAME));

    // --- logs: in-memory batch exporter (+ 옵션 OTLP batch exporter) ---
    let log_exporter = InMemoryLogExporter::default();
    let mut logger_builder = SdkLoggerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(log_exporter.clone());
    if otlp_on {
        let otlp_logs = opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .build()
            .expect("build OTLP log exporter");
        logger_builder = logger_builder.with_batch_exporter(otlp_logs);
    }
    let logger_provider = logger_builder.build();
    // tracing 이벤트를 OTel 로그로 흘리는 bridge. 자기 텔레메트리가 다시 로그를 만드는 피드백 루프를
    // 막기 위해 전송 계층(hyper/tonic/h2/reqwest) 로그는 끈다.
    let otel_log_layer = OpenTelemetryTracingBridge::new(&logger_provider).with_filter(
        EnvFilter::new("info")
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("tonic=off".parse().unwrap())
            .add_directive("h2=off".parse().unwrap())
            .add_directive("reqwest=off".parse().unwrap())
            // transport/exporters는 client 미연결 시 전송할 signal 전체를 info!로 덤프한다.
            // 이 로그가 다시 OTel 로그로 잡혀 다음 주기에 수집·재전송되면 payload가 주기마다
            // 중첩 직렬화되어 기하급수적으로 불어나는 피드백 루프가 생기므로 여기서 끊는다.
            .add_directive("skid_monitor_agent::transport=off".parse().unwrap())
            .add_directive("skid_monitor_agent::exporters=off".parse().unwrap()),
    );

    // --- subscriber: 콘솔(fmt) + span 브리지 + log 브리지 ---
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));
    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_span_layer)
        .with(otel_log_layer)
        .init();

    TelemetryGuard {
        metric_exporter,
        span_exporter,
        log_exporter,
        resource,
        meter_provider,
        tracer_provider,
        logger_provider,
    }
}

impl TelemetryGuard {
    /// 모인 배치를 flush 하고 provider를 종료한다. 프로세스 종료 직전에 호출해야 손실이 없다.
    pub fn shutdown(self) {
        let _ = self.tracer_provider.shutdown();
        let _ = self.meter_provider.shutdown();
        let _ = self.logger_provider.shutdown();
    }
}
