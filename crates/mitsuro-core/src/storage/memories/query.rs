use super::model::{
    AgentMemory, MemoryNamespace, MemorySensitivity, MemorySource, MemoryStatus, MemoryType,
};

pub(super) const MEMORY_SELECT_COLUMNS: &str =
    "id, memory_type, title, content, project_dir, user_id, created_at, updated_at, \
     canonical_key, namespace, namespace_id, status, source, source_session_id, \
     source_message_id, confidence, sensitivity, pinned, supersedes_id, \
     last_accessed_at, access_count";

pub(super) fn row_to_memory(row: &rusqlite::Row) -> AgentMemory {
    let type_str: String = row.get(1).unwrap_or_default();
    let namespace: String = row.get(9).unwrap_or_else(|_| "shared".to_string());
    let status: String = row.get(11).unwrap_or_else(|_| "active".to_string());
    let source: String = row.get(12).unwrap_or_else(|_| "legacy".to_string());
    let sensitivity: String = row.get(16).unwrap_or_else(|_| "normal".to_string());
    AgentMemory {
        id: row.get(0).unwrap_or_default(),
        memory_type: type_str.parse().unwrap_or(MemoryType::Project),
        title: row.get(2).unwrap_or_default(),
        content: row.get(3).unwrap_or_default(),
        project_dir: row.get::<_, Option<String>>(4).unwrap_or_default(),
        user_id: row.get::<_, Option<String>>(5).unwrap_or_default(),
        created_at: row.get(6).unwrap_or_default(),
        updated_at: row.get(7).unwrap_or_default(),
        canonical_key: row.get::<_, Option<String>>(8).unwrap_or_default(),
        namespace: namespace.parse().unwrap_or(MemoryNamespace::Shared),
        namespace_id: row.get::<_, Option<String>>(10).unwrap_or_default(),
        status: status.parse().unwrap_or(MemoryStatus::Active),
        source: source.parse().unwrap_or(MemorySource::Legacy),
        source_session_id: row.get::<_, Option<String>>(13).unwrap_or_default(),
        source_message_id: row.get::<_, Option<String>>(14).unwrap_or_default(),
        confidence: row.get(15).unwrap_or(1.0),
        sensitivity: sensitivity.parse().unwrap_or(MemorySensitivity::Normal),
        pinned: row.get::<_, i64>(17).unwrap_or_default() != 0,
        supersedes_id: row.get::<_, Option<String>>(18).unwrap_or_default(),
        last_accessed_at: row.get::<_, Option<String>>(19).unwrap_or_default(),
        access_count: row.get(20).unwrap_or_default(),
    }
}

pub(super) fn build_list_query(
    memory_type: Option<MemoryType>,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> (String, Vec<String>) {
    let mut sql =
        format!("SELECT {MEMORY_SELECT_COLUMNS} FROM agent_memories WHERE status = 'active'");
    let mut bound: Vec<String> = Vec::new();

    if let Some(mt) = memory_type {
        bound.push(mt.as_str().to_string());
        sql.push_str(&format!(" AND memory_type = ?{}", bound.len()));
    }

    if let Some(pd) = project_dir {
        bound.push(pd.to_string());
        sql.push_str(&format!(
            " AND (project_dir = ?{} OR project_dir IS NULL)",
            bound.len()
        ));
    } else {
        sql.push_str(" AND project_dir IS NULL");
    }

    if let Some(uid) = user_id {
        bound.push(uid.to_string());
        sql.push_str(&format!(
            " AND (user_id = ?{} OR user_id IS NULL)",
            bound.len()
        ));
    } else {
        sql.push_str(" AND user_id IS NULL");
    }

    sql.push_str(" ORDER BY updated_at DESC");
    (sql, bound)
}
