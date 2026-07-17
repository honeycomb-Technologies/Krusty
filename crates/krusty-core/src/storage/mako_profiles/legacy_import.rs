use std::path::Path;

use super::model::{
    MakoCrewProfileDocumentKind, MakoCrewProfileSeed, MakoProfileDocumentKind,
    MakoProfileMergeResult, MakoProfileOwner, MakoProfileSeed,
};
use super::store::{MakoProfileStore, MakoProfileStoreError};
use crate::storage::{MakoCrewDocumentKind, MakoHomeDocumentKind, MakoHomeProfile};

const DEFAULT_CREW_SLUGS: &[&str] = &["builder", "researcher", "reviewer"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakoLegacyImportResult {
    pub merge: MakoProfileMergeResult,
    pub excluded_memory: bool,
    pub excluded_crew_memory_count: usize,
}

/// Build the canonical profile seed without reading or writing a filesystem.
///
/// The seed intentionally contains identity/personality documents only. Legacy
/// memory is imported by the canonical memory subsystem, never as active
/// profile instructions.
pub fn default_profile_seed() -> MakoProfileSeed {
    let documents = [
        (MakoProfileDocumentKind::Soul, MakoHomeDocumentKind::Soul),
        (
            MakoProfileDocumentKind::Identity,
            MakoHomeDocumentKind::Identity,
        ),
        (MakoProfileDocumentKind::User, MakoHomeDocumentKind::User),
        (
            MakoProfileDocumentKind::Heartbeat,
            MakoHomeDocumentKind::Heartbeat,
        ),
        (
            MakoProfileDocumentKind::Channels,
            MakoHomeDocumentKind::Channels,
        ),
    ]
    .into_iter()
    .map(|(profile_kind, legacy_kind)| (profile_kind, legacy_kind.default_content().to_string()))
    .collect();

    let crew = DEFAULT_CREW_SLUGS
        .iter()
        .map(|slug| MakoCrewProfileSeed {
            slug: (*slug).to_string(),
            documents: vec![
                (
                    MakoCrewProfileDocumentKind::Identity,
                    MakoCrewDocumentKind::Identity.default_content(slug),
                ),
                (
                    MakoCrewProfileDocumentKind::Soul,
                    MakoCrewDocumentKind::Soul.default_content(slug),
                ),
            ],
        })
        .collect();

    MakoProfileSeed { documents, crew }
}

impl MakoProfileStore {
    /// Create any missing default identity documents without overwriting edits.
    pub fn bootstrap_defaults(
        &self,
        owner: &MakoProfileOwner,
    ) -> Result<MakoProfileMergeResult, MakoProfileStoreError> {
        self.merge_missing(owner, &default_profile_seed())
    }

    /// Import a legacy filesystem home into the local database profile.
    ///
    /// `MAKO_MEMORY.md` and crew `MEMORY.md` remain available to a future
    /// canonical-memory importer but are deliberately excluded from identity.
    pub fn import_local_legacy_home(
        &self,
        owner: &MakoProfileOwner,
        mako_home: &Path,
    ) -> Result<MakoLegacyImportResult, MakoProfileStoreError> {
        if !owner.is_local() {
            return Err(MakoProfileStoreError::LegacyImportRequiresLocalOwner);
        }

        let legacy = MakoHomeProfile::load_from(mako_home);
        let mut seed = MakoProfileSeed::default();
        push_document(
            &mut seed,
            MakoProfileDocumentKind::Soul,
            legacy.soul.as_ref(),
        );
        push_document(
            &mut seed,
            MakoProfileDocumentKind::Identity,
            legacy.identity.as_ref(),
        );
        push_document(
            &mut seed,
            MakoProfileDocumentKind::User,
            legacy.user.as_ref(),
        );
        push_document(
            &mut seed,
            MakoProfileDocumentKind::Heartbeat,
            legacy.heartbeat.as_ref(),
        );
        push_document(
            &mut seed,
            MakoProfileDocumentKind::Channels,
            legacy.channels.as_ref(),
        );

        let excluded_memory = legacy.memory.is_some();
        let mut excluded_crew_memory_count = 0usize;
        for member in &legacy.crew {
            let mut documents = Vec::new();
            if let Some(identity) = member.identity.as_ref() {
                documents.push((
                    MakoCrewProfileDocumentKind::Identity,
                    identity.content.clone(),
                ));
            }
            if let Some(soul) = member.soul.as_ref() {
                documents.push((MakoCrewProfileDocumentKind::Soul, soul.content.clone()));
            }
            excluded_crew_memory_count += usize::from(member.memory.is_some());
            if !documents.is_empty() {
                seed.crew.push(MakoCrewProfileSeed {
                    slug: member.slug.clone(),
                    documents,
                });
            }
        }

        Ok(MakoLegacyImportResult {
            merge: self.merge_missing(owner, &seed)?,
            excluded_memory,
            excluded_crew_memory_count,
        })
    }
}

fn push_document(
    seed: &mut MakoProfileSeed,
    kind: MakoProfileDocumentKind,
    document: Option<&crate::storage::MakoHomeDocument>,
) {
    if let Some(document) = document {
        seed.documents.push((kind, document.content.clone()));
    }
}
