use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use serde_json::Value;

use super::model::{DelegatedRunRecord, DelegatedRunRole, DelegatedRunScope, DelegatedRunSnapshot};
use crate::agent::subagent::AgentCapability;
use crate::agent::DelegatedRunStage;

pub(super) fn delegated_stage_str(stage: DelegatedRunStage) -> &'static str {
    match stage {
        DelegatedRunStage::Created => "created",
        DelegatedRunStage::Running => "running",
        DelegatedRunStage::Synthesizing => "synthesizing",
        DelegatedRunStage::Complete => "complete",
        DelegatedRunStage::Degraded => "degraded",
        DelegatedRunStage::Failed => "failed",
        DelegatedRunStage::Cancelled => "cancelled",
    }
}

pub(super) fn parse_stage(value: &str) -> rusqlite::Result<DelegatedRunStage> {
    match value {
        "created" => Ok(DelegatedRunStage::Created),
        "running" => Ok(DelegatedRunStage::Running),
        "synthesizing" => Ok(DelegatedRunStage::Synthesizing),
        "complete" => Ok(DelegatedRunStage::Complete),
        "degraded" => Ok(DelegatedRunStage::Degraded),
        "failed" => Ok(DelegatedRunStage::Failed),
        "cancelled" => Ok(DelegatedRunStage::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            Type::Text,
            format!("unknown delegated stage: {}", other).into(),
        )),
    }
}

pub(super) fn parse_datetime(value: String) -> rusqlite::Result<DateTime<Utc>> {
    value
        .parse::<DateTime<Utc>>()
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, err.into()))
}

pub(super) fn row_to_delegated_run(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<DelegatedRunRecord> {
    let role = DelegatedRunRole::from_str(&row.get::<_, String>(3)?).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, "invalid delegated role".into())
    })?;
    let stage = parse_stage(&row.get::<_, String>(4)?)?;
    let target_scope_json: String = row.get(10)?;
    let target_scope = serde_json::from_str::<Vec<DelegatedRunScope>>(&target_scope_json)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(10, Type::Text, err.into()))?;
    let snapshot = row
        .get::<_, Option<String>>(11)?
        .map(|value| serde_json::from_str::<DelegatedRunSnapshot>(&value))
        .transpose()
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(11, Type::Text, err.into()))?;
    let artifact = row
        .get::<_, Option<String>>(12)?
        .map(|value| serde_json::from_str::<Value>(&value))
        .transpose()
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(12, Type::Text, err.into()))?;
    let capabilities_json: String = row.get(18)?;
    let capabilities =
        serde_json::from_str::<std::collections::BTreeSet<AgentCapability>>(&capabilities_json)
            .map_err(|err| rusqlite::Error::FromSqlConversionFailure(18, Type::Text, err.into()))?;

    Ok(DelegatedRunRecord {
        delegated_run_id: row.get(0)?,
        parent_session_id: row.get(1)?,
        parent_tool_call_id: row.get(2)?,
        role,
        stage,
        provider: row.get(5)?,
        model: row.get(6)?,
        resumable: row.get::<_, i64>(7)? != 0,
        resumed_from_run_id: row.get(8)?,
        target_scope_key: row.get(9)?,
        target_scope,
        snapshot,
        artifact,
        human_review: row.get(13)?,
        created_at: parse_datetime(row.get(14)?)?,
        updated_at: parse_datetime(row.get(15)?)?,
        completed_at: row
            .get::<_, Option<String>>(16)?
            .map(parse_datetime)
            .transpose()?,
        child_name: row.get(17)?,
        capabilities,
    })
}
