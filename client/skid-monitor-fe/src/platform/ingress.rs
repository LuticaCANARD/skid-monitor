#[cfg(target_arch = "wasm32")]
use skid_monitor_core::TenantId;
use skid_protocol::protocol::Signal;

#[cfg(any(target_arch = "wasm32", test))]
mod cloud;
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
    #[cfg(target_arch = "wasm32")]
    BrowserStorageScope(BrowserStorageScope),
    #[cfg(not(target_arch = "wasm32"))]
    ExtensionError(String),
}

/// Browser persistence boundary selected by the active ingress mode.
///
/// Cloud storage is unavailable while authentication is pending, then scoped
/// by both canonical client API endpoint and the tenant proven by a one-time
/// stream ticket. The ticket and access token never become part of this value.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BrowserStorageScope {
    Legacy,
    CloudPending,
    Cloud {
        endpoint: String,
        tenant_id: TenantId,
    },
}

#[cfg(target_arch = "wasm32")]
impl BrowserStorageScope {
    pub(crate) fn storage_key(&self, base: &str) -> Option<String> {
        match self {
            Self::Legacy => Some(base.to_string()),
            Self::CloudPending => None,
            Self::Cloud {
                endpoint,
                tenant_id,
            } => Some(cloud::browser_storage_key(base, endpoint, *tenant_id)),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Legacy => "legacy browser ingress".to_string(),
            Self::CloudPending => "cloud authentication pending".to_string(),
            Self::Cloud {
                endpoint,
                tenant_id,
            } => format!("cloud tenant {tenant_id} via {endpoint}"),
        }
    }
}
