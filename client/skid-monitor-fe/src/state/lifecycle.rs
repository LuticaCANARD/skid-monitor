use super::DashboardState;
use crate::alert::AlertStore;
use crate::edge::{EdgeSignalDecorations, edge_key};
use crate::model::{NodeSummary, SignalCounters, Status};
#[cfg(target_arch = "wasm32")]
use crate::platform::BrowserStorageScope;
use crate::platform::{Ingress, IngressControl, IngressMessage};
use crate::storage::StateStorage;
use skid_protocol::protocol::Signal;
use std::collections::{BTreeMap, VecDeque};
use web_time::Instant;

impl DashboardState {
    pub(crate) fn new() -> Self {
        let storage_init = StateStorage::start();
        let avatar_profile = storage_init
            .avatar_profile
            .and_then(|profile| profile.normalized().ok())
            .unwrap_or_default();
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
            alerts_enabled: true,
            avatar_profile,
            avatar_profile_revision: 0,
            avatar_model_revision: 0,
            #[cfg(not(target_arch = "wasm32"))]
            pending_avatar_profile: None,
            storage: storage_init.storage,
            #[cfg(target_arch = "wasm32")]
            browser_storage_scope: storage_init.browser_scope,
            listeners: Default::default(),
            ingress_control: None,
        };

        if let Some(message) = storage_init.message {
            state.push_event("storage", message);
        }

        state
    }

    /// Wires up the channel used to ask the running receiver loop to manage
    /// client ingress listeners at runtime.
    pub(crate) fn set_ingress_control(&mut self, ingress_control: IngressControl) {
        self.ingress_control = Some(ingress_control);
    }

    pub(crate) fn drain_ingress(&mut self, ingress: &mut Ingress) {
        while let Some(message) = ingress.try_next() {
            match message {
                IngressMessage::Listening(addrs) => self.observe_listening(addrs),
                IngressMessage::Signal { listener, signal } => {
                    self.observe_signal_message(listener, signal);
                }
                IngressMessage::Error { listener, error } => {
                    self.observe_receiver_error(listener.as_deref(), error);
                }
                #[cfg(target_arch = "wasm32")]
                IngressMessage::BrowserStorageScope(scope) => {
                    self.observe_browser_storage_scope(scope);
                }
                #[cfg(not(target_arch = "wasm32"))]
                IngressMessage::ExtensionError(error) => {
                    self.push_event("extension", error.clone());
                    if self.alerts_enabled {
                        let change = self.alerts.observe_extension_error(&error);
                        self.push_alert_change(change);
                    }
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
            return Err("ingress is required".to_string());
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
            format!("registered observation agent {node} via {endpoint}"),
        );

        Ok(key)
    }

    pub(crate) fn remove_agent(&mut self, key: &str) -> Result<(), String> {
        let Some(node) = self.nodes.remove(key) else {
            return Err("agent not found".to_string());
        };

        self.edge_decorations.remove(key);
        self.forget_edge(key);
        self.push_event(
            "agent",
            format!(
                "removed observation agent {} via {}",
                node.node, node.endpoint
            ),
        );

        Ok(())
    }

    pub(crate) fn add_listener(&mut self, addr: &str) -> Result<(), String> {
        let addr = addr.trim();
        if addr.is_empty() {
            return Err("listen address is required".to_string());
        }
        if self.listeners.contains(addr) {
            return Err(format!("listener {addr} is already active"));
        }

        let Some(ingress_control) = &self.ingress_control else {
            return Err("ingress control is unavailable".to_string());
        };
        ingress_control.add(addr.to_string())?;
        self.push_event(
            "receiver",
            format!("requested ingress activation for {addr}"),
        );
        Ok(())
    }

    pub(crate) fn remove_listener(&mut self, addr: &str) -> Result<(), String> {
        let addr = addr.trim();
        if addr.is_empty() {
            return Err("listen address is required".to_string());
        }
        if !self.listeners.contains(addr) {
            return Err(format!("listener {addr} is not active"));
        }

        let Some(ingress_control) = &self.ingress_control else {
            return Err("ingress control is unavailable".to_string());
        };
        ingress_control.remove(addr.to_string())?;
        self.push_event("receiver", format!("requested ingress removal for {addr}"));
        Ok(())
    }

    fn observe_listening(&mut self, addrs: Vec<String>) {
        let label = listener_status_label(&addrs);
        self.listeners = addrs.iter().cloned().collect();
        self.listening_label = Some(label.clone());
        self.status = Status::Listening(label);
        self.push_event("receiver", listener_event_message(&addrs));
        if self.alerts_enabled {
            for addr in addrs {
                let change = self
                    .alerts
                    .observe_receiver_recovered(&addr, "receiver is listening");
                self.push_alert_change(change);
            }
        }
    }

    fn observe_signal_message(&mut self, listener: String, signal: Signal) {
        if let Some(label) = &self.listening_label {
            self.status = Status::Listening(label.clone());
        }
        if self.alerts_enabled {
            let change = self
                .alerts
                .observe_receiver_recovered(&listener, "receiver received a signal");
            self.push_alert_change(change);
        }
        self.ingest_signal(&listener, signal);
    }

    fn observe_receiver_error(&mut self, listener: Option<&str>, error: String) {
        let source = listener.unwrap_or("receiver");
        self.push_event("error", error.clone());
        if self.alerts_enabled {
            let change = self.alerts.observe_receiver_error(source, &error);
            self.push_alert_change(change);
        }
        if self.listening_label.is_none() {
            self.status = Status::Error(error);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn observe_browser_storage_scope(&mut self, scope: BrowserStorageScope) {
        if self.browser_storage_scope == scope {
            return;
        }

        let label = scope.label();
        let restored = match &self.storage {
            Some(storage) => storage.activate_browser_scope(scope.clone()),
            None => Err("browser state storage is unavailable".to_string()),
        };
        self.browser_storage_scope = scope;
        self.avatar_profile_revision = self.avatar_profile_revision.wrapping_add(1);
        self.avatar_model_revision = self.avatar_model_revision.wrapping_add(1);

        // Dashboard models are not tenant-partitioned in memory. Replace the
        // complete signal-derived view before accepting records for a newly
        // authenticated cloud tenant (or while authentication is pending).
        self.counters = SignalCounters::default();
        self.events.clear();
        self.metrics.clear();
        self.metric_history.clear();
        self.nodes.clear();
        self.edge_decorations = EdgeSignalDecorations::default();
        self.alerts = AlertStore::default();
        self.status = self
            .listening_label
            .clone()
            .map(Status::Listening)
            .unwrap_or(Status::Starting);

        match restored {
            Ok(restored) => {
                self.avatar_profile = restored.avatar_profile.unwrap_or_default();
                let restored_nodes = self.edge_decorations.restore(restored.restored_edges);
                self.nodes = restored_nodes
                    .into_iter()
                    .map(|node| (edge_key(&node.endpoint, &node.node), node))
                    .collect();
                self.push_event("storage", format!("browser state scope changed to {label}"));
                if let Some(warning) = restored.warning {
                    self.push_event("storage", warning);
                }
            }
            Err(error) => {
                self.avatar_profile = Default::default();
                self.push_event(
                    "error",
                    format!("browser state scope changed to {label}, but restore failed: {error}"),
                );
            }
        }
    }
}

fn listener_status_label(addrs: &[String]) -> String {
    let count = addrs.len();
    match count {
        0 => "receiver idle".to_string(),
        1 => "receiver ready (1 listener)".to_string(),
        _ => format!("receiver ready ({count} listeners)"),
    }
}

fn listener_event_message(addrs: &[String]) -> String {
    match addrs {
        [] => "receiver has no active listeners".to_string(),
        [addr] => format!("receiver listening on {addr}"),
        _ => format!("receiver listening on {} ingress addresses", addrs.len()),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use skid_monitor_client::receiver_loop::ReceiverControl;
    use std::sync::mpsc;

    fn empty_state() -> DashboardState {
        DashboardState {
            status: Status::Starting,
            listening_label: None,
            counters: SignalCounters::default(),
            events: VecDeque::new(),
            metrics: VecDeque::new(),
            metric_history: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edge_decorations: EdgeSignalDecorations::default(),
            alerts: AlertStore::default(),
            alerts_enabled: true,
            avatar_profile: Default::default(),
            avatar_profile_revision: 0,
            avatar_model_revision: 0,
            pending_avatar_profile: None,
            storage: None,
            listeners: Default::default(),
            ingress_control: None,
        }
    }

    #[test]
    fn registering_agent_does_not_bind_a_listener_implicitly() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.set_ingress_control(IngressControl::from_sender(tx));

        state
            .register_agent("127.0.0.1:9300", "agent-a", "skid-monitor-agent")
            .unwrap();

        assert!(rx.try_recv().is_err());
        assert!(state.listeners.is_empty());
    }

    #[test]
    fn listener_bind_is_an_explicit_receiver_control_request() {
        let (tx, rx) = mpsc::channel();
        let mut state = empty_state();
        state.set_ingress_control(IngressControl::from_sender(tx));

        state.add_listener("127.0.0.1:9300").unwrap();

        match rx.try_recv().unwrap() {
            ReceiverControl::AddListener(addr) => assert_eq!(addr, "127.0.0.1:9300"),
            ReceiverControl::RemoveListener(addr) => {
                panic!("expected AddListener, got RemoveListener({addr})")
            }
        }
    }

    #[test]
    fn character_profile_update_is_validated_and_applied_atomically() {
        let db_path = std::env::temp_dir().join(format!(
            "skid-monitor-fe-state-profile-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let storage_init = StateStorage::start_at(db_path.clone());
        let mut state = empty_state();
        state.storage = Some(storage_init.storage.expect("test state storage"));
        let original = state.avatar_profile().clone();
        let invalid = crate::model::AvatarReactionProfile {
            model_name: "  ".to_string(),
            ..original.clone()
        };

        assert!(state.set_avatar_profile(invalid).is_err());
        assert_eq!(state.avatar_profile(), &original);

        let valid = crate::model::AvatarReactionProfile {
            model_name: "  Operator Cat  ".to_string(),
            ..original
        };
        state.set_avatar_profile(valid).expect("valid profile");
        assert!(state.avatar_profile_save_pending());
        assert_eq!(state.avatar_profile().model_name, "Skid");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while state.avatar_profile_save_pending() {
            if let Some(result) = state.poll_avatar_profile_save() {
                result.expect("profile save result");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "profile save did not complete"
            );
            std::thread::yield_now();
        }

        assert_eq!(state.avatar_profile().model_name, "Operator Cat");
        assert!(
            state
                .events()
                .back()
                .is_some_and(|event| event.message.contains("Operator Cat"))
        );

        drop(state);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_path.display()));
        }
    }

    #[test]
    fn character_profile_is_not_applied_when_storage_is_unavailable() {
        let mut state = empty_state();
        let original = state.avatar_profile().clone();
        let profile = crate::model::AvatarReactionProfile {
            model_name: "Ephemeral Cat".to_string(),
            ..original.clone()
        };

        let error = state
            .set_avatar_profile(profile)
            .expect_err("profile must remain durable or be rejected");

        assert!(error.contains("storage is unavailable"));
        assert_eq!(state.avatar_profile(), &original);
    }
}
