use crate::{AgentId, SignalEnvelope};
use serde::{Deserialize, Serialize};
use skid_protocol::otlp::tonic::metrics::v1::metric;
use skid_protocol::protocol::Signal;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignalCounters {
    pub metric_batches: u64,
    pub metric_points: u64,
    pub trace_batches: u64,
    pub spans: u64,
    pub log_batches: u64,
    pub log_records: u64,
}

impl SignalCounters {
    fn observe(&mut self, signal: &Signal) {
        match signal {
            Signal::Metrics(request) => {
                self.metric_batches = self.metric_batches.saturating_add(1);
                let count = request
                    .resource_metrics
                    .iter()
                    .flat_map(|resource| &resource.scope_metrics)
                    .flat_map(|scope| &scope.metrics)
                    .map(metric_data_point_count)
                    .sum::<u64>();
                self.metric_points = self.metric_points.saturating_add(count);
            }
            Signal::Traces(request) => {
                self.trace_batches = self.trace_batches.saturating_add(1);
                let count = request
                    .resource_spans
                    .iter()
                    .flat_map(|resource| &resource.scope_spans)
                    .map(|scope| scope.spans.len() as u64)
                    .sum::<u64>();
                self.spans = self.spans.saturating_add(count);
            }
            Signal::Logs(request) => {
                self.log_batches = self.log_batches.saturating_add(1);
                let count = request
                    .resource_logs
                    .iter()
                    .flat_map(|resource| &resource.scope_logs)
                    .map(|scope| scope.log_records.len() as u64)
                    .sum::<u64>();
                self.log_records = self.log_records.saturating_add(count);
            }
        }
    }
}

fn metric_data_point_count(metric: &skid_protocol::otlp::tonic::metrics::v1::Metric) -> u64 {
    match metric.data.as_ref() {
        Some(metric::Data::Gauge(data)) => data.data_points.len() as u64,
        Some(metric::Data::Sum(data)) => data.data_points.len() as u64,
        Some(metric::Data::Histogram(data)) => data.data_points.len() as u64,
        Some(metric::Data::ExponentialHistogram(data)) => data.data_points.len() as u64,
        Some(metric::Data::Summary(data)) => data.data_points.len() as u64,
        None => 0,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSignalProjection {
    pub agent_id: AgentId,
    pub last_sequence: u64,
    pub last_received_at_unix_nano: u64,
    pub counters: SignalCounters,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignalProjection {
    pub counters: SignalCounters,
    pub agents: BTreeMap<AgentId, AgentSignalProjection>,
}

impl SignalProjection {
    pub fn observe(&mut self, envelope: &SignalEnvelope) {
        self.counters.observe(&envelope.payload);
        let agent = self
            .agents
            .entry(envelope.agent_id.clone())
            .or_insert_with(|| AgentSignalProjection {
                agent_id: envelope.agent_id.clone(),
                last_sequence: 0,
                last_received_at_unix_nano: 0,
                counters: SignalCounters::default(),
            });
        agent.last_sequence = agent.last_sequence.max(envelope.sequence);
        agent.last_received_at_unix_nano = agent
            .last_received_at_unix_nano
            .max(envelope.received_at_unix_nano);
        agent.counters.observe(&envelope.payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SignalScope;
    use skid_protocol::metrics::{Metric, MetricKind, Source, export_metrics};

    #[test]
    fn projection_is_shared_by_solo_and_cloud_envelopes() {
        let signal = Signal::Metrics(export_metrics(
            vec![Metric {
                name: "system.cpu.usage".to_string(),
                value: 2.0,
                source: Source::System,
                unit: None,
                kind: MetricKind::Gauge,
                attributes: Vec::new(),
            }],
            "agent",
            "test",
        ));
        let envelope = SignalEnvelope::new(
            SignalScope::Solo,
            AgentId::new("agent-a").unwrap(),
            3,
            9,
            signal,
        );
        let mut projection = SignalProjection::default();
        projection.observe(&envelope);

        assert_eq!(projection.counters.metric_batches, 1);
        assert_eq!(projection.counters.metric_points, 1);
        assert_eq!(projection.agents[&envelope.agent_id].last_sequence, 3);
    }

    #[test]
    fn metric_projection_counts_data_points_not_metric_descriptors() {
        let mut request = export_metrics(
            vec![Metric {
                name: "system.cpu.usage".to_string(),
                value: 2.0,
                source: Source::System,
                unit: None,
                kind: MetricKind::Gauge,
                attributes: Vec::new(),
            }],
            "agent",
            "test",
        );
        let data = request.resource_metrics[0].scope_metrics[0].metrics[0]
            .data
            .as_mut()
            .expect("test metric data");
        let metric::Data::Gauge(gauge) = data else {
            panic!("test helper should build a gauge");
        };
        gauge.data_points.push(gauge.data_points[0].clone());

        let envelope = SignalEnvelope::new(
            SignalScope::Solo,
            AgentId::new("agent-a").unwrap(),
            4,
            10,
            Signal::Metrics(request),
        );
        let mut projection = SignalProjection::default();
        projection.observe(&envelope);

        assert_eq!(projection.counters.metric_batches, 1);
        assert_eq!(projection.counters.metric_points, 2);
        assert_eq!(
            projection.agents[&envelope.agent_id].counters.metric_points,
            2
        );
    }
}
