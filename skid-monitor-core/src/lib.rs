//! Deployment-neutral signal service contracts.
//!
//! PostgreSQL-backed cloud services use these contracts directly, and Solo
//! adapters can adopt the same boundary without taking a PostgreSQL dependency.
//! The native Solo frontend currently keeps its existing `DashboardState`
//! projection path. Transport, persistence, and authorization remain adapters
//! around this crate.

mod envelope;
mod projection;
mod store;

pub use envelope::{
    AgentId, EventId, IdError, SignalCursor, SignalEnvelope, SignalKind, SignalScope, TenantId,
};
pub use projection::{AgentSignalProjection, SignalCounters, SignalProjection};
pub use store::{AppendOutcome, SignalReader, SignalRecord, SignalWriter, StoreFuture};
