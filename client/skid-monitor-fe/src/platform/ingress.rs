use skid_protocol::protocol::Signal;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{Ingress, IngressControl};
#[cfg(target_arch = "wasm32")]
pub(crate) use web::{Ingress, IngressControl};

pub(crate) enum IngressMessage {
    Listening(Vec<String>),
    Signal {
        listener: String,
        signal: Signal,
    },
    Error {
        listener: Option<String>,
        error: String,
    },
    #[cfg(not(target_arch = "wasm32"))]
    ExtensionError(String),
}
