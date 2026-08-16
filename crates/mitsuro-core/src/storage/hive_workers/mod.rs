//! First-class Hive Worker identities.
//!
//! A Worker is the durable product identity for Hive: persona documents, a
//! frozen provider/model choice, an autonomy policy, and a private DM session
//! whose controller is the Worker's serialized execution lane. Crew profiles
//! remain the transitional slug-keyed source; `resolve_worker_for_crew_slug`
//! bridges legacy `crew_slug` call sites until they migrate to worker ids.

mod model;
pub(crate) mod store;

#[cfg(test)]
mod tests;

pub use model::{
    display_name_from_slug, HiveWorker, HiveWorkerAutonomy, HiveWorkerDocument,
    HiveWorkerDocumentKind, HiveWorkerProfileUpdate, HiveWorkerStatus, NewHiveWorker,
};
pub use store::{load_worker_with_conn, HiveWorkerStore};
