use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::storage::{MemoryAclScope, MemoryNamespace};

/// Immutable access scope frozen when a report is created.
///
/// Owner-shared reports are visible to ordinary Chat/Code and the primary
/// Hive surface for the exact owner. Worker-private reports are additionally
/// keyed by both the Worker's durable id and its memory namespace id; neither
/// value is re-derived from a mutable DM or group-lane binding at read time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportScope {
    memory_namespace: MemoryNamespace,
    namespace_id: Option<String>,
    acl_scope: MemoryAclScope,
    source_worker_id: Option<String>,
}

impl ReportScope {
    pub fn owner_shared() -> Self {
        Self {
            memory_namespace: MemoryNamespace::Shared,
            namespace_id: None,
            acl_scope: MemoryAclScope::Owner,
            source_worker_id: None,
        }
    }

    pub fn worker_private(
        source_worker_id: impl Into<String>,
        namespace_id: impl Into<String>,
    ) -> Result<Self, String> {
        let scope = Self {
            memory_namespace: MemoryNamespace::Crew,
            namespace_id: Some(namespace_id.into()),
            acl_scope: MemoryAclScope::Worker,
            source_worker_id: Some(source_worker_id.into()),
        };
        scope.validate()?;
        Ok(scope)
    }

    pub fn memory_namespace(&self) -> MemoryNamespace {
        self.memory_namespace
    }

    pub fn namespace_id(&self) -> Option<&str> {
        self.namespace_id.as_deref()
    }

    pub fn acl_scope(&self) -> MemoryAclScope {
        self.acl_scope
    }

    pub fn source_worker_id(&self) -> Option<&str> {
        self.source_worker_id.as_deref()
    }

    pub(super) fn from_storage(
        memory_namespace: MemoryNamespace,
        namespace_id: Option<String>,
        acl_scope: MemoryAclScope,
        source_worker_id: Option<String>,
    ) -> Result<Self, String> {
        let scope = Self {
            memory_namespace,
            namespace_id,
            acl_scope,
            source_worker_id,
        };
        scope.validate()?;
        Ok(scope)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        match (
            self.memory_namespace,
            self.namespace_id.as_deref(),
            self.acl_scope,
            self.source_worker_id.as_deref(),
        ) {
            (MemoryNamespace::Shared, None, MemoryAclScope::Owner, None) => Ok(()),
            (
                MemoryNamespace::Crew,
                Some(namespace_id),
                MemoryAclScope::Worker,
                Some(worker_id),
            ) if !namespace_id.trim().is_empty() && !worker_id.trim().is_empty() => Ok(()),
            _ => Err("invalid report memory namespace and ACL combination".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub project_dir: Option<String>,
    pub content: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: String,
    /// Exact owner copied from the source session at creation time.
    pub owner_user_id: Option<String>,
    pub scope: ReportScope,
}

pub struct CreateReportInput<'a> {
    pub title: &'a str,
    pub session_id: &'a str,
    pub project_dir: Option<&'a str>,
    pub report_root: Option<&'a Path>,
    pub content: &'a str,
    pub summary: &'a str,
    pub tags: &'a [String],
    pub sources: &'a [String],
    /// Caller-declared scope. The store recomputes the authoritative scope
    /// from the persisted session inside the insert transaction and rejects a
    /// mismatch instead of trusting this value.
    pub scope: ReportScope,
}
