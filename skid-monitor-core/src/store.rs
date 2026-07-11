use crate::{SignalCursor, SignalEnvelope, SignalScope};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

pub type StoreFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendOutcome {
    pub cursor: SignalCursor,
    pub inserted: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SignalRecord {
    pub cursor: SignalCursor,
    pub envelope: SignalEnvelope,
}

pub trait SignalWriter: Send + Sync {
    type Error;

    fn append<'a>(
        &'a self,
        envelope: &'a SignalEnvelope,
    ) -> StoreFuture<'a, AppendOutcome, Self::Error>;
}

pub trait SignalReader: Send + Sync {
    type Error;

    fn load_after<'a>(
        &'a self,
        scope: SignalScope,
        after: SignalCursor,
        limit: usize,
    ) -> StoreFuture<'a, Vec<SignalRecord>, Self::Error>;
}
