use super::IngressMessage;
use skid_monitor_client::receiver_loop::{
    ReceiverControl, ReceiverMessage, spawn_receiver_managed_with_notify,
};
use std::sync::mpsc::{Receiver, Sender};

pub(crate) struct Ingress {
    rx: Receiver<ReceiverMessage>,
    control: IngressControl,
}

#[derive(Clone)]
pub(crate) struct IngressControl {
    tx: Sender<ReceiverControl>,
}

impl Ingress {
    pub(crate) fn start(ctx: &eframe::egui::Context) -> Self {
        let ctx = ctx.clone();
        let (rx, tx) = spawn_receiver_managed_with_notify(move || ctx.request_repaint());
        Self {
            rx,
            control: IngressControl { tx },
        }
    }

    pub(crate) fn control(&self) -> IngressControl {
        self.control.clone()
    }

    pub(crate) fn try_next(&mut self) -> Option<IngressMessage> {
        self.rx.try_recv().ok().map(Into::into)
    }
}

impl IngressControl {
    #[cfg(test)]
    pub(crate) fn from_sender(tx: Sender<ReceiverControl>) -> Self {
        Self { tx }
    }

    pub(crate) fn add(&self, endpoint: String) -> Result<(), String> {
        self.tx
            .send(ReceiverControl::AddListener(endpoint))
            .map_err(|error| format!("failed to request ingress bind: {error}"))
    }

    pub(crate) fn remove(&self, endpoint: String) -> Result<(), String> {
        self.tx
            .send(ReceiverControl::RemoveListener(endpoint))
            .map_err(|error| format!("failed to request ingress removal: {error}"))
    }
}

impl From<ReceiverMessage> for IngressMessage {
    fn from(message: ReceiverMessage) -> Self {
        match message {
            ReceiverMessage::Listening(addrs) => Self::Listening(addrs),
            ReceiverMessage::Signal { listener, signal } => Self::Signal { listener, signal },
            ReceiverMessage::Error { listener, error } => Self::Error { listener, error },
            ReceiverMessage::ExtensionError(error) => Self::ExtensionError(error),
        }
    }
}
