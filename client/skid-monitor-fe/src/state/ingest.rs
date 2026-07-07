use super::DashboardState;
use crate::config;
use crate::edge::edge_key;
use crate::model::{MetricSample, NodeSummary};
use crate::signal::metric_samples;
use crate::utils::push_capped;
use skid_protocol::protocol::Signal;
use std::time::Instant;

impl DashboardState {
    pub(in crate::state) fn ingest_signal(&mut self, listener: &str, signal: Signal) {
        match &signal {
            Signal::Metrics(request) => {
                self.counters.metrics += 1;
                let samples = metric_samples(request, listener);
                let sample_count = samples.len();
                self.counters.metric_points += sample_count;
                for sample in samples {
                    self.observe_metric_sample(&sample);
                    if let Some(value) = sample.numeric {
                        push_capped(
                            self.metric_history
                                .entry(sample.trend_key.clone())
                                .or_default(),
                            value,
                            config::MAX_HISTORY_POINTS,
                        );
                    }
                    if self.alerts_enabled {
                        let change = self.alerts.observe_metric(&sample);
                        self.push_alert_change(change);
                        self.update_edge_alert_severity(&sample);
                    }
                    push_capped(&mut self.metrics, sample, config::MAX_METRICS);
                }
                self.push_event(
                    "metrics",
                    format!(
                        "received {} metric points from {} resources via {}",
                        sample_count,
                        request.resource_metrics.len(),
                        listener
                    ),
                );
            }
            Signal::Traces(request) => {
                let count = request
                    .resource_spans
                    .iter()
                    .flat_map(|resource| &resource.scope_spans)
                    .map(|scope| scope.spans.len())
                    .sum::<usize>();
                self.counters.traces += 1;
                self.counters.spans += count;
                self.observe_signal_items(listener, "traces", count);
                self.push_event("traces", format!("received {count} spans via {listener}"));
            }
            Signal::Logs(request) => {
                let count = request
                    .resource_logs
                    .iter()
                    .flat_map(|resource| &resource.scope_logs)
                    .map(|scope| scope.log_records.len())
                    .sum::<usize>();
                self.counters.logs += 1;
                self.counters.log_records += count;
                self.observe_signal_items(listener, "logs", count);
                self.push_event(
                    "logs",
                    format!("received {count} log records via {listener}"),
                );
            }
        }
    }

    fn observe_metric_sample(&mut self, sample: &MetricSample) {
        let key = edge_key(&sample.endpoint, &sample.node);
        let edge = {
            let entry = self.nodes.entry(key).or_insert_with(|| NodeSummary {
                node: sample.node.clone(),
                endpoint: sample.endpoint.clone(),
                source: sample.source.clone(),
                service: sample.service.clone(),
                metric_points: 0,
                spans: 0,
                log_records: 0,
                last_metric: String::new(),
                last_value: String::new(),
                last_seen: Instant::now(),
            });

            entry.node = sample.node.clone();
            entry.endpoint = sample.endpoint.clone();
            entry.source = sample.source.clone();
            entry.service = sample.service.clone();
            entry.metric_points += 1;
            entry.last_metric = sample.name.clone();
            entry.last_value = sample.value.clone();
            entry.last_seen = Instant::now();
            self.edge_decorations.observe_node(entry, "metrics")
        };
        self.persist_edge(edge);
    }

    fn observe_signal_items(&mut self, listener: &str, kind: &str, count: usize) {
        let key = edge_key(listener, listener);
        let edge = {
            let entry = self.nodes.entry(key).or_insert_with(|| NodeSummary {
                node: listener.to_string(),
                endpoint: listener.to_string(),
                source: kind.to_string(),
                service: config::METRIC_EMPTY_FIELD.to_string(),
                metric_points: 0,
                spans: 0,
                log_records: 0,
                last_metric: String::new(),
                last_value: String::new(),
                last_seen: Instant::now(),
            });

            entry.source = kind.to_string();
            match kind {
                "traces" => entry.spans += count,
                "logs" => entry.log_records += count,
                _ => {}
            }
            entry.last_metric = kind.to_string();
            entry.last_value = count.to_string();
            entry.last_seen = Instant::now();
            self.edge_decorations.observe_node(entry, kind)
        };
        self.persist_edge(edge);
    }

    fn update_edge_alert_severity(&mut self, sample: &MetricSample) {
        let severity = self.alerts.highest_for_node(&sample.endpoint, &sample.node);
        if let Some(edge) =
            self.edge_decorations
                .set_node_severity(&sample.endpoint, &sample.node, severity)
        {
            self.persist_edge(edge);
        }
    }
}
