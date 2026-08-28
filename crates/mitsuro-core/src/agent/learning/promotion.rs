use anyhow::{bail, Result};

use crate::storage::{
    CanonicalMemoryInput, LearningCandidate, LearningKind, LearningSensitivity, MemoryAclScope,
    MemoryNamespace, MemorySensitivity, MemorySource, MemoryType,
};

use super::LearningScope;

pub(super) fn scope_for_candidate(candidate: &LearningCandidate) -> LearningScope {
    match candidate.kind {
        LearningKind::UserPreference
        | LearningKind::UserCorrection
        | LearningKind::RelationshipContext => LearningScope::User,
        LearningKind::ProjectFact | LearningKind::Procedure => LearningScope::Project,
        LearningKind::Forget => {
            if candidate.project_dir.is_some() {
                LearningScope::Project
            } else {
                LearningScope::User
            }
        }
    }
}

pub(super) fn canonical_input_for_candidate(
    candidate: &LearningCandidate,
) -> Result<CanonicalMemoryInput> {
    validate_candidate_memory_scope(candidate)?;
    if candidate.sensitivity != LearningSensitivity::Normal {
        bail!("sensitive or prohibited candidates cannot become canonical memory");
    }

    let (memory_type, title_kind, scope) = match candidate.kind {
        LearningKind::UserPreference => (MemoryType::User, "Preference", LearningScope::User),
        LearningKind::UserCorrection => (MemoryType::Feedback, "Correction", LearningScope::User),
        LearningKind::ProjectFact => (MemoryType::Project, "Project fact", LearningScope::Project),
        LearningKind::Procedure => (MemoryType::Project, "Procedure", LearningScope::Project),
        LearningKind::RelationshipContext => {
            (MemoryType::User, "Relationship", LearningScope::User)
        }
        LearningKind::Forget => bail!("forget candidates are tombstones, not canonical memories"),
    };

    match scope {
        LearningScope::User if candidate.project_dir.is_some() => {
            bail!("user-scoped candidates cannot target a project")
        }
        LearningScope::Project if candidate.project_dir.is_none() => {
            bail!("project-scoped candidates require an exact project")
        }
        LearningScope::User | LearningScope::Project => {}
    }

    let mut input = CanonicalMemoryInput::new(
        memory_type,
        candidate.canonical_key.clone(),
        format!("{title_kind}: {}", readable_key(&candidate.canonical_key)),
        candidate.proposed_content.clone(),
    );
    input.project_dir = candidate.project_dir.clone();
    input.user_id = candidate.user_id.clone();
    input.namespace = candidate.memory_namespace;
    input.namespace_id = candidate.memory_namespace_id.clone();
    input.acl_scope = candidate.memory_acl_scope;
    input.source = if candidate.explicit {
        MemorySource::User
    } else {
        MemorySource::Agent
    };
    input.source_session_id = Some(candidate.evidence_session_id.clone());
    input.source_message_id = Some(candidate.evidence_message_id.to_string());
    input.confidence = candidate.confidence;
    input.sensitivity = MemorySensitivity::Normal;
    Ok(input)
}

pub(super) fn validate_candidate_memory_scope(candidate: &LearningCandidate) -> Result<()> {
    if !candidate.memory_scope_resolved {
        bail!("learning candidate memory scope was not resolved at evidence ingest");
    }
    match candidate.memory_namespace {
        MemoryNamespace::Crew => {
            if candidate.memory_namespace_id.is_none()
                || candidate.memory_acl_scope != MemoryAclScope::Worker
            {
                bail!("Worker learning candidate has an invalid private memory scope");
            }
        }
        MemoryNamespace::Shared | MemoryNamespace::Hive => {
            if candidate.memory_namespace_id.is_some()
                || candidate.memory_acl_scope != MemoryAclScope::Owner
            {
                bail!("owner learning candidate has an invalid memory scope");
            }
        }
    }
    Ok(())
}

fn readable_key(key: &str) -> String {
    key.chars()
        .map(|character| {
            if matches!(character, '.' | '_' | '-') {
                ' '
            } else {
                character
            }
        })
        .collect()
}
