use std::collections::BTreeSet;

use rusqlite::{params, OptionalExtension, Transaction};
use thiserror::Error;

use crate::storage::Database;

use super::model::{
    MakoCrewProfileDocumentKind, MakoCrewProfileSnapshot, MakoProfileDocument,
    MakoProfileDocumentKind, MakoProfileMergeResult, MakoProfileOwner, MakoProfileOwnerError,
    MakoProfileSeed, MakoProfileSnapshot,
};

/// Profile documents are durable prompt inputs and API response fields. Keep
/// each document independently bounded even though the final prompt renderer
/// also applies an aggregate budget.
pub const MAX_MAKO_PROFILE_DOCUMENT_BYTES: usize = 64 * 1024;

pub struct MakoProfileStore {
    db: Database,
}

#[derive(Debug, Error)]
pub enum MakoProfileStoreError {
    #[error(transparent)]
    InvalidOwner(#[from] MakoProfileOwnerError),
    #[error("Hive profile content must not be empty")]
    EmptyContent,
    #[error(
        "Hive profile content exceeds the {MAX_MAKO_PROFILE_DOCUMENT_BYTES}-byte document limit"
    )]
    ContentTooLarge,
    #[error("invalid Hive agent slug: {0}")]
    InvalidCrewSlug(String),
    #[error("Hive profile revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: i64, actual: i64 },
    #[error("Hive profile owner collision for profile id {profile_id}")]
    OwnerCollision { profile_id: String },
    #[error("Hive profile {0} was not found")]
    ProfileNotFound(String),
    #[error("invalid stored Hive profile document kind: {0}")]
    InvalidStoredDocumentKind(String),
    #[error("invalid stored Hive agent document kind: {0}")]
    InvalidStoredCrewDocumentKind(String),
    #[error("legacy autonomous homes may only be imported into the local Hive profile")]
    LegacyImportRequiresLocalOwner,
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}

impl MakoProfileStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn load(
        &self,
        owner: &MakoProfileOwner,
    ) -> Result<Option<MakoProfileSnapshot>, MakoProfileStoreError> {
        let profile_id = owner.profile_id();
        let snapshot = load_snapshot_from_connection(self.db.conn(), &profile_id)?;
        if snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.user_id.as_deref() != owner.user_id())
        {
            return Err(MakoProfileStoreError::OwnerCollision { profile_id });
        }
        Ok(snapshot)
    }

    pub fn get_or_create(
        &self,
        owner: &MakoProfileOwner,
    ) -> Result<MakoProfileSnapshot, MakoProfileStoreError> {
        let tx = self.db.conn().unchecked_transaction()?;
        ensure_profile_tx(&tx, owner)?;
        tx.commit()?;
        self.load(owner)?
            .ok_or_else(|| MakoProfileStoreError::ProfileNotFound(owner.profile_id()))
    }

    pub fn update_document(
        &self,
        owner: &MakoProfileOwner,
        kind: MakoProfileDocumentKind,
        content: &str,
        expected_revision: i64,
    ) -> Result<MakoProfileSnapshot, MakoProfileStoreError> {
        let content = normalized_content(content)?;
        let profile_id = owner.profile_id();
        let tx = self.db.conn().unchecked_transaction()?;
        ensure_profile_tx(&tx, owner)?;
        let actual_revision = profile_revision_tx(&tx, &profile_id)?;
        require_revision(expected_revision, actual_revision)?;

        tx.execute(
            "INSERT INTO mako_profile_documents (profile_id, kind, content, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(profile_id, kind) DO UPDATE SET
                 content = excluded.content,
                 updated_at = excluded.updated_at",
            params![profile_id, kind.as_str(), content],
        )?;
        advance_profile_revision_tx(&tx, &profile_id, actual_revision)?;
        tx.commit()?;

        self.load(owner)?
            .ok_or_else(|| MakoProfileStoreError::ProfileNotFound(owner.profile_id()))
    }

    pub fn update_crew_document(
        &self,
        owner: &MakoProfileOwner,
        slug: &str,
        kind: MakoCrewProfileDocumentKind,
        content: &str,
        expected_revision: i64,
    ) -> Result<MakoProfileSnapshot, MakoProfileStoreError> {
        validate_crew_slug(slug)?;
        let content = normalized_content(content)?;
        let profile_id = owner.profile_id();
        let tx = self.db.conn().unchecked_transaction()?;
        ensure_profile_tx(&tx, owner)?;
        let actual_revision = profile_revision_tx(&tx, &profile_id)?;
        require_revision(expected_revision, actual_revision)?;

        tx.execute(
            "INSERT OR IGNORE INTO mako_crew_profiles
                (profile_id, slug, revision, created_at, updated_at)
             VALUES (?1, ?2, 0, datetime('now'), datetime('now'))",
            params![profile_id, slug],
        )?;
        tx.execute(
            "INSERT INTO mako_crew_documents (profile_id, slug, kind, content, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(profile_id, slug, kind) DO UPDATE SET
                 content = excluded.content,
                 updated_at = excluded.updated_at",
            params![profile_id, slug, kind.as_str(), content],
        )?;
        tx.execute(
            "UPDATE mako_crew_profiles
             SET revision = revision + 1, updated_at = datetime('now')
             WHERE profile_id = ?1 AND slug = ?2",
            params![profile_id, slug],
        )?;
        advance_profile_revision_tx(&tx, &profile_id, actual_revision)?;
        tx.commit()?;

        self.load(owner)?
            .ok_or_else(|| MakoProfileStoreError::ProfileNotFound(owner.profile_id()))
    }

    /// Insert only missing profile documents in one transaction.
    ///
    /// Existing user-authored content is never overwritten. The aggregate
    /// profile revision advances once if anything was imported and remains
    /// unchanged on an idempotent replay.
    pub fn merge_missing(
        &self,
        owner: &MakoProfileOwner,
        seed: &MakoProfileSeed,
    ) -> Result<MakoProfileMergeResult, MakoProfileStoreError> {
        let profile_id = owner.profile_id();
        let tx = self.db.conn().unchecked_transaction()?;
        ensure_profile_tx(&tx, owner)?;
        let original_revision = profile_revision_tx(&tx, &profile_id)?;
        let mut inserted_documents = Vec::new();
        let mut inserted_crew_documents = Vec::new();
        let mut changed_crew = BTreeSet::new();

        for (kind, content) in &seed.documents {
            let content = normalized_content(content)?;
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO mako_profile_documents
                    (profile_id, kind, content, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                params![profile_id, kind.as_str(), content],
            )?;
            if inserted > 0 {
                inserted_documents.push(*kind);
            }
        }

        for crew in &seed.crew {
            validate_crew_slug(&crew.slug)?;
            tx.execute(
                "INSERT OR IGNORE INTO mako_crew_profiles
                    (profile_id, slug, revision, created_at, updated_at)
                 VALUES (?1, ?2, 0, datetime('now'), datetime('now'))",
                params![profile_id, crew.slug],
            )?;
            for (kind, content) in &crew.documents {
                let content = normalized_content(content)?;
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO mako_crew_documents
                        (profile_id, slug, kind, content, updated_at)
                     VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                    params![profile_id, crew.slug, kind.as_str(), content],
                )?;
                if inserted > 0 {
                    changed_crew.insert(crew.slug.clone());
                    inserted_crew_documents.push((crew.slug.clone(), *kind));
                }
            }
        }

        for slug in &changed_crew {
            tx.execute(
                "UPDATE mako_crew_profiles
                 SET revision = revision + 1, updated_at = datetime('now')
                 WHERE profile_id = ?1 AND slug = ?2",
                params![profile_id, slug],
            )?;
        }

        if !inserted_documents.is_empty() || !inserted_crew_documents.is_empty() {
            advance_profile_revision_tx(&tx, &profile_id, original_revision)?;
        }
        tx.commit()?;

        let snapshot = self
            .load(owner)?
            .ok_or_else(|| MakoProfileStoreError::ProfileNotFound(owner.profile_id()))?;
        Ok(MakoProfileMergeResult {
            snapshot,
            inserted_documents,
            inserted_crew_documents,
        })
    }
}

fn normalized_content(content: &str) -> Result<&str, MakoProfileStoreError> {
    let content = content.trim();
    if content.is_empty() {
        return Err(MakoProfileStoreError::EmptyContent);
    }
    if content.len() > MAX_MAKO_PROFILE_DOCUMENT_BYTES {
        return Err(MakoProfileStoreError::ContentTooLarge);
    }
    Ok(content)
}

fn validate_crew_slug(slug: &str) -> Result<(), MakoProfileStoreError> {
    if super::super::mako_home::is_valid_crew_slug(slug) {
        Ok(())
    } else {
        Err(MakoProfileStoreError::InvalidCrewSlug(slug.to_string()))
    }
}

fn require_revision(expected: i64, actual: i64) -> Result<(), MakoProfileStoreError> {
    if expected == actual {
        Ok(())
    } else {
        Err(MakoProfileStoreError::RevisionConflict { expected, actual })
    }
}

fn ensure_profile_tx(
    tx: &Transaction<'_>,
    owner: &MakoProfileOwner,
) -> Result<(), MakoProfileStoreError> {
    let profile_id = owner.profile_id();
    tx.execute(
        "INSERT OR IGNORE INTO mako_profiles
            (id, user_id, revision, created_at, updated_at)
         VALUES (?1, ?2, 0, datetime('now'), datetime('now'))",
        params![profile_id, owner.user_id()],
    )?;

    let stored_user_id = tx
        .query_row(
            "SELECT user_id FROM mako_profiles WHERE id = ?1",
            [&profile_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    if stored_user_id
        .as_ref()
        .and_then(|stored_user_id| stored_user_id.as_deref())
        != owner.user_id()
    {
        return Err(MakoProfileStoreError::OwnerCollision { profile_id });
    }
    Ok(())
}

fn profile_revision_tx(
    tx: &Transaction<'_>,
    profile_id: &str,
) -> Result<i64, MakoProfileStoreError> {
    tx.query_row(
        "SELECT revision FROM mako_profiles WHERE id = ?1",
        [profile_id],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| MakoProfileStoreError::ProfileNotFound(profile_id.to_string()))
}

fn advance_profile_revision_tx(
    tx: &Transaction<'_>,
    profile_id: &str,
    current_revision: i64,
) -> Result<(), MakoProfileStoreError> {
    let changed = tx.execute(
        "UPDATE mako_profiles
         SET revision = revision + 1, updated_at = datetime('now')
         WHERE id = ?1 AND revision = ?2",
        params![profile_id, current_revision],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        let actual = profile_revision_tx(tx, profile_id)?;
        Err(MakoProfileStoreError::RevisionConflict {
            expected: current_revision,
            actual,
        })
    }
}

fn load_snapshot_from_connection(
    conn: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Option<MakoProfileSnapshot>, MakoProfileStoreError> {
    let profile = conn
        .query_row(
            "SELECT user_id, revision FROM mako_profiles WHERE id = ?1",
            [profile_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((user_id, revision)) = profile else {
        return Ok(None);
    };

    let mut snapshot = MakoProfileSnapshot {
        profile_id: profile_id.to_string(),
        user_id,
        revision,
        soul: None,
        identity: None,
        user: None,
        heartbeat: None,
        channels: None,
        crew: Vec::new(),
    };

    let mut document_stmt = conn.prepare(
        "SELECT kind, content, updated_at
         FROM mako_profile_documents
         WHERE profile_id = ?1
         ORDER BY kind",
    )?;
    let document_rows = document_stmt.query_map([profile_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in document_rows {
        let (kind, content, updated_at) = row?;
        let kind = MakoProfileDocumentKind::parse(&kind)
            .ok_or_else(|| MakoProfileStoreError::InvalidStoredDocumentKind(kind.clone()))?;
        let document = Some(MakoProfileDocument {
            kind,
            content,
            updated_at,
        });
        match kind {
            MakoProfileDocumentKind::Soul => snapshot.soul = document,
            MakoProfileDocumentKind::Identity => snapshot.identity = document,
            MakoProfileDocumentKind::User => snapshot.user = document,
            MakoProfileDocumentKind::Heartbeat => snapshot.heartbeat = document,
            MakoProfileDocumentKind::Channels => snapshot.channels = document,
        }
    }

    let mut crew_stmt = conn.prepare(
        "SELECT slug, revision
         FROM mako_crew_profiles
         WHERE profile_id = ?1
         ORDER BY slug",
    )?;
    let crew_rows = crew_stmt.query_map([profile_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in crew_rows {
        let (slug, revision) = row?;
        let mut member = MakoCrewProfileSnapshot {
            slug: slug.clone(),
            revision,
            ..Default::default()
        };
        let mut crew_document_stmt = conn.prepare(
            "SELECT kind, content, updated_at
             FROM mako_crew_documents
             WHERE profile_id = ?1 AND slug = ?2
             ORDER BY kind",
        )?;
        let crew_document_rows =
            crew_document_stmt.query_map(params![profile_id, slug], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
        for document_row in crew_document_rows {
            let (kind, content, updated_at) = document_row?;
            let kind = MakoCrewProfileDocumentKind::parse(&kind).ok_or_else(|| {
                MakoProfileStoreError::InvalidStoredCrewDocumentKind(kind.clone())
            })?;
            let document = Some(MakoProfileDocument {
                kind,
                content,
                updated_at,
            });
            match kind {
                MakoCrewProfileDocumentKind::Identity => member.identity = document,
                MakoCrewProfileDocumentKind::Soul => member.soul = document,
            }
        }
        snapshot.crew.push(member);
    }

    Ok(Some(snapshot))
}
