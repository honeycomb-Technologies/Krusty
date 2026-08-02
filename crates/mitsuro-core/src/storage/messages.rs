//! Message persistence storage
//!
//! Handles saving and loading messages for sessions.

mod model;
mod store;
#[cfg(test)]
mod tests;

pub use model::StoredMessageRecord;
pub use store::MessageStore;
