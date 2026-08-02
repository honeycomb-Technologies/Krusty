//! Database-owned Hive identity profiles.
//!
//! A profile is keyed by authenticated user identity, never by the currently
//! selected workspace. The filesystem-backed `hive_home` module remains a
//! compatibility source for one-time local imports.

mod legacy_import;
mod model;
mod store;

#[cfg(test)]
mod tests;

pub use legacy_import::{default_profile_seed, HiveLegacyImportResult};
pub use model::{
    HiveCrewProfileDocumentKind, HiveCrewProfileSeed, HiveCrewProfileSnapshot, HiveProfileDocument,
    HiveProfileDocumentKind, HiveProfileMergeResult, HiveProfileOwner, HiveProfileOwnerError,
    HiveProfileSeed, HiveProfileSnapshot,
};
pub use store::{HiveProfileStore, HiveProfileStoreError, MAX_HIVE_PROFILE_DOCUMENT_BYTES};
