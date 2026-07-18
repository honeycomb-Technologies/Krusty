//! Database-owned Mako identity profiles.
//!
//! A profile is keyed by authenticated user identity, never by the currently
//! selected workspace. The filesystem-backed `mako_home` module remains a
//! compatibility source for one-time local imports.

mod legacy_import;
mod model;
mod store;

#[cfg(test)]
mod tests;

pub use legacy_import::{default_profile_seed, MakoLegacyImportResult};
pub use model::{
    MakoCrewProfileDocumentKind, MakoCrewProfileSeed, MakoCrewProfileSnapshot, MakoProfileDocument,
    MakoProfileDocumentKind, MakoProfileMergeResult, MakoProfileOwner, MakoProfileOwnerError,
    MakoProfileSeed, MakoProfileSnapshot,
};
pub use store::{MakoProfileStore, MakoProfileStoreError, MAX_MAKO_PROFILE_DOCUMENT_BYTES};
