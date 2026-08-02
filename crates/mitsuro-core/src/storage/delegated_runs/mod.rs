//! Persisted delegated run records for first-class subagent lifecycle tracking.

mod codec;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub use model::{
    normalize_scope_key, DelegatedRunAgentSnapshot, DelegatedRunRecord, DelegatedRunRole,
    DelegatedRunScope, DelegatedRunSnapshot, DelegatedRunStartInput,
};
pub use store::DelegatedRunStore;
