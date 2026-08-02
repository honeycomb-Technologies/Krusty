use std::path::Path;

use super::model::{
    HiveCrewProfileDocumentKind, HiveCrewProfileSeed, HiveProfileDocumentKind,
    HiveProfileMergeResult, HiveProfileOwner, HiveProfileSeed,
};
use super::store::{HiveProfileStore, HiveProfileStoreError};
use crate::storage::{HiveCrewDocumentKind, HiveHomeDocumentKind, HiveHomeProfile};

const DEFAULT_CREW_SLUGS: &[&str] = &["builder", "researcher", "reviewer"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiveLegacyImportResult {
    pub merge: HiveProfileMergeResult,
    pub excluded_memory: bool,
    pub excluded_crew_memory_count: usize,
}

/// Build the canonical profile seed without reading or writing a filesystem.
///
/// The seed intentionally contains identity/personality documents only. Legacy
/// memory is imported by the canonical memory subsystem, never as active
/// profile instructions.
pub fn default_profile_seed() -> HiveProfileSeed {
    let documents = [
        (HiveProfileDocumentKind::Soul, HiveHomeDocumentKind::Soul),
        (
            HiveProfileDocumentKind::Identity,
            HiveHomeDocumentKind::Identity,
        ),
        (HiveProfileDocumentKind::User, HiveHomeDocumentKind::User),
        (
            HiveProfileDocumentKind::Heartbeat,
            HiveHomeDocumentKind::Heartbeat,
        ),
        (
            HiveProfileDocumentKind::Channels,
            HiveHomeDocumentKind::Channels,
        ),
    ]
    .into_iter()
    .map(|(profile_kind, legacy_kind)| (profile_kind, legacy_kind.default_content().to_string()))
    .collect();

    let crew = DEFAULT_CREW_SLUGS
        .iter()
        .map(|slug| HiveCrewProfileSeed {
            slug: (*slug).to_string(),
            documents: vec![
                (
                    HiveCrewProfileDocumentKind::Identity,
                    HiveCrewDocumentKind::Identity.default_content(slug),
                ),
                (
                    HiveCrewProfileDocumentKind::Soul,
                    HiveCrewDocumentKind::Soul.default_content(slug),
                ),
            ],
        })
        .collect();

    HiveProfileSeed { documents, crew }
}

impl HiveProfileStore {
    /// Create any missing default identity documents without overwriting edits.
    pub fn bootstrap_defaults(
        &self,
        owner: &HiveProfileOwner,
    ) -> Result<HiveProfileMergeResult, HiveProfileStoreError> {
        self.merge_missing(owner, &default_profile_seed())
    }

    /// Import a legacy filesystem home into the local database profile.
    ///
    /// `HIVE_MEMORY.md` and crew `MEMORY.md` remain available to a future
    /// canonical-memory importer but are deliberately excluded from identity.
    pub fn import_local_legacy_home(
        &self,
        owner: &HiveProfileOwner,
        hive_home: &Path,
    ) -> Result<HiveLegacyImportResult, HiveProfileStoreError> {
        if !owner.is_local() {
            return Err(HiveProfileStoreError::LegacyImportRequiresLocalOwner);
        }

        let legacy = HiveHomeProfile::load_from(hive_home);
        let mut seed = HiveProfileSeed::default();
        push_document(
            &mut seed,
            HiveProfileDocumentKind::Soul,
            legacy.soul.as_ref(),
        );
        push_document(
            &mut seed,
            HiveProfileDocumentKind::Identity,
            legacy.identity.as_ref(),
        );
        push_document(
            &mut seed,
            HiveProfileDocumentKind::User,
            legacy.user.as_ref(),
        );
        push_document(
            &mut seed,
            HiveProfileDocumentKind::Heartbeat,
            legacy.heartbeat.as_ref(),
        );
        push_document(
            &mut seed,
            HiveProfileDocumentKind::Channels,
            legacy.channels.as_ref(),
        );

        let excluded_memory = legacy.memory.is_some();
        let mut excluded_crew_memory_count = 0usize;
        for member in &legacy.crew {
            let mut documents = Vec::new();
            if let Some(identity) = member.identity.as_ref() {
                documents.push((
                    HiveCrewProfileDocumentKind::Identity,
                    identity.content.clone(),
                ));
            }
            if let Some(soul) = member.soul.as_ref() {
                documents.push((HiveCrewProfileDocumentKind::Soul, soul.content.clone()));
            }
            excluded_crew_memory_count += usize::from(member.memory.is_some());
            if !documents.is_empty() {
                seed.crew.push(HiveCrewProfileSeed {
                    slug: member.slug.clone(),
                    documents,
                });
            }
        }

        Ok(HiveLegacyImportResult {
            merge: self.merge_missing(owner, &seed)?,
            excluded_memory,
            excluded_crew_memory_count,
        })
    }
}

fn push_document(
    seed: &mut HiveProfileSeed,
    kind: HiveProfileDocumentKind,
    document: Option<&crate::storage::HiveHomeDocument>,
) {
    if let Some(document) = document {
        seed.documents.push((kind, document.content.clone()));
    }
}
