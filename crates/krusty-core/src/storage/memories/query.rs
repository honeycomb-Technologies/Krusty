use super::model::{AgentMemory, MemoryType};

pub(super) fn row_to_memory(row: &rusqlite::Row) -> AgentMemory {
    let type_str: String = row.get(1).unwrap_or_default();
    AgentMemory {
        id: row.get(0).unwrap_or_default(),
        memory_type: type_str.parse().unwrap_or(MemoryType::Project),
        title: row.get(2).unwrap_or_default(),
        content: row.get(3).unwrap_or_default(),
        project_dir: row.get(4).ok(),
        user_id: row.get(5).ok(),
        created_at: row.get(6).unwrap_or_default(),
        updated_at: row.get(7).unwrap_or_default(),
    }
}

pub(super) fn build_list_query(
    memory_type: Option<MemoryType>,
    project_dir: Option<&str>,
    user_id: Option<&str>,
) -> (String, Vec<String>) {
    let mut sql = String::from(
        "SELECT id, memory_type, title, content, project_dir, user_id, created_at, updated_at
         FROM agent_memories WHERE 1=1",
    );
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
    }

    sql.push_str(" ORDER BY updated_at DESC");
    (sql, bound)
}
