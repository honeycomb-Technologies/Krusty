//! Immutable, user-scoped conversation recall for Mako.
//!
//! Episodes are deliberately separate from canonical model history. Compaction
//! is allowed to rewrite `messages`, while episodes preserve the small subset
//! of user/assistant text that is safe and useful for later recall.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{ConversationEpisode, EpisodeSearch};
pub use store::EpisodeStore;
