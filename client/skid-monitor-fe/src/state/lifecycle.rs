use super::DashboardState;
use crate::alert::AlertStore;
use crate::edge::{EdgeSignalDecorations, edge_key};
use crate::model::{NodeSummary, SignalCounters, Status};
use crate::storage::StateStorage;
use skid_monitor_client::receiver_loop::ReceiverMessage;
use skid_protocol::protocol::Signal;
use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::Instant;

impl DashboardState {
    pub(crate) fn new() -> Self {
        let storage_init = StateStorage::start();
        let mut edge_decorations = EdgeSignalDecorations::default();
        let restored_nodes = edge_decorations.restore(storage_init.restored_edges);
        let nodes = restored_nodes
            .into_iter()
            .map(|node| (edge_key(&node.endpoint, &node.node), node))
            .collect();

        let mut state = Self {
            status: Status::Starting,
            listening_label: None,
            counters: SignalCounters::default(),
            events: VecDeque::new(),
            metrics: VecDeque::new(),
            metric_history: BTreeMap::new(),
            nodes,
            edge_decorations,
            alerts: AlertStore::default(),
            storage: storage_init.storage,
        };

        if let Some(message) = storage_init.message {
            state.push_event("storage", message);
        }

        state
    }

    pub(crate) fn drain_messages(&mut self, rx: &Receiver<ReceiverMessage>) {
        while let Ok(message) = rx.try_recv() {
            match message {
                ReceiverMessage::Listening(addrs) => self.observe_listening(addrs),
                ReceiverMessage::Signal { listener, signal } => {
                    self.observe_signal_message(listener, signal);
                }
                ReceiverMessage::Error { listener, error } => {
                    self.observe_receiver_error(listener.as_deref(), error);
                }
                ReceiverMessage::ExtensionError(error) => {
                    self.push_event("extension", error.clone());
                    let change = self.alerts.observe_extension_error(&error);
                    self.push_alert_change(change);
                }
            }
        }
    }

    pub(crate) fn register_agent(
        &mut self,
        endpoint: &str,
        node: &str,
        service: &str,
    ) -> Result<String, String> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err("endpoint is required".to_string());
        }

        let node = node.trim();
        let node = if node.is_empty() { endpoint } else { node };
        let service = service.trim();
        let service = if service.is_empty() {
            "skid-monitor-agent"
        } else {
            service
        };
        let key = edge_key(endpoint, node);
        if self.nodes.contains_key(&key) {
            return Err(format!("{node} is already registered"));
        }

        let summary = NodeSummary {
            node: node.to_string(),
            endpoint: endpoint.to_string(),
            source: "manual".to_string(),
            service: service.to_string(),
            metric_points: 0,
            spans: 0,
            log_records: 0,
            last_metric: "registered".to_string(),
            last_value: "pending".to_string(),
            last_seen: Instant::now(),
        };
        let edge = self.edge_decorations.observe_node(&summary, "manual");
        self.nodes.insert(key.clone(), summary);
        self.persist_edge(edge);
        self.push_event(
            "agent",
            format!("registered observation agent {node} at {endpoint}"),
        );

        Ok(key)
    }

    fn observe_listening(&mut self, addrs: Vec<String>) {
        let label = listener_status_label(&addrs);
        self.listening_label = Some(label.clone());
        self.status = Status::Listening(label);
        self.push_event("receiver", "receiver ready");
        for addr in addrs {
            let change = self
                .alerts
                .observe_receiver_recovered(&addr, "receiver is listening");
            self.push_alert_change(change);
        }
    }

    fn observe_signal_message(&mut self, listener: String, signal: Signal) {
        if let Some(label) = &self.listening_label {
            self.status = Status::Listening(label.clone());
        }
        let change = self
            .alerts
            .observe_receiver_recovered(&listener, "receiver received a signal");
        self.push_alert_change(change);
        self.ingest_signal(&listener, signal);
    }

    fn observe_receiver_error(&mut self, listener: Option<&str>, error: String) {
        let source = listener.unwrap_or("receiver");
        self.push_event("error", error.clone());
        let change = self.alerts.observe_receiver_error(source, &error);
        if self.listening_label.is_none() {
            self.status = Status::Error(error);
        }
        self.push_alert_change(change);
    }
}

fn listener_status_label(addrs: &[String]) -> String {
    match addrs {
        [] => "receiver idle".to_string(),
        _ => "receiver ready".to_string(),
    }
}
