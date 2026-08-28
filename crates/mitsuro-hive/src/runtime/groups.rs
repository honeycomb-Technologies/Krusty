//! Group turn engine: durable fan-out of one room message across member
//! Workers.
//!
//! `group_message` runs inside the daemon's idempotent mutation transaction:
//! it appends the trigger message with a per-group sequence, snapshots the
//! group policy into a `hive_group_turns` row, resolves targets (explicit
//! override or server-side mention parsing), and queues member runs on each
//! Worker's own controller lane. Workbench queues every target immediately
//! with slot-scoped concurrency keys so the pump's claim loop enforces the
//! group's parallelism cap; roundtable queues only the first speaker and the
//! pump advances the rotation as each run reaches a terminal state; direct
//! routes to the assignee. One member's failure never cancels siblings — the
//! turn aggregates per-member outcomes and finishes completed/partial/failed.

use mitsuro_core::hive::RetryPolicy;
use mitsuro_core::storage::hive_groups::{
    self, GroupMentionTarget, HiveGroup, HiveGroupExecutionMode, HiveGroupStatus, HiveGroupTurn,
    HiveGroupTurnPolicy, HiveGroupTurnStatus, NewHiveGroupMessage, MAX_HIVE_GROUP_MESSAGE_BYTES,
};
use mitsuro_core::storage::{
    load_group_worker_lane_with_conn, upsert_group_worker_lane_with_conn,
    HiveRunExecutionContextV1, HiveWorker, HiveWorkerIntroductionStore, HiveWorkerStatus,
    NewHiveGroupWorkerLane, WorkerConversationLane, WorkerRunOrigin,
};
use mitsuro_hive_protocol::{Actor, GroupMessageCommand, GroupTurnResponse, ResponsePayload};
use rusqlite::{params, Transaction};
use serde_json::{json, Map, Value};

use super::handler::ack;
use super::persistence::{
    append_event, get_or_create_controller, require_controller, require_owned_session, Mutation,
    PersistedEvent, RuntimeStoreError,
};

/// Trigger excerpt bound inside member run objectives.
const MAX_TRIGGER_EXCERPT_BYTES: usize = 2 * 1024;
const MEMBER_RUN_MAX_ATTEMPTS: u32 = 5;

/// Run statuses that still hold or will hold an execution slot.
pub(super) const NON_TERMINAL_RUN_STATUSES: &[&str] = &[
    "queued",
    "leased",
    "running",
    "sleeping",
    "retry_wait",
    "awaiting_input",
    "recovery_required",
];

pub(super) fn group_message(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    command: GroupMessageCommand,
    idempotency_key: &str,
    origin: WorkerRunOrigin,
) -> Result<Mutation, RuntimeStoreError> {
    let message = command.message.trim();
    if message.is_empty() {
        return Err(RuntimeStoreError::Invalid("message is empty".into()));
    }
    if message.len() > MAX_HIVE_GROUP_MESSAGE_BYTES {
        return Err(RuntimeStoreError::Invalid(format!(
            "message exceeds {MAX_HIVE_GROUP_MESSAGE_BYTES} bytes"
        )));
    }

    let group = require_owned_group(tx, actor, &command.group_id)?;
    if group.status != HiveGroupStatus::Active {
        return Err(RuntimeStoreError::StateConflict(
            "the group is archived; unarchive it before sending".into(),
        ));
    }
    if group.execution_mode == HiveGroupExecutionMode::Roundtable && group.max_rounds == 0 {
        return Err(RuntimeStoreError::Invalid(
            "roundtable groups need max_rounds >= 1".into(),
        ));
    }

    let roster = hive_groups::load_member_workers(tx, &group.id)
        .map_err(RuntimeStoreError::Internal)?
        .into_iter()
        .filter(|worker| worker.status != HiveWorkerStatus::Archived)
        .collect::<Vec<_>>();
    if roster.is_empty() {
        return Err(RuntimeStoreError::StateConflict(
            "the group has no active member Workers".into(),
        ));
    }

    let explicit_worker_targeting = group_message_has_explicit_worker_target(
        &roster,
        message,
        command.mentions_override.as_deref(),
    );
    let resolved_targets = resolve_targets(
        &group,
        &roster,
        message,
        command.mentions_override.as_deref(),
    )?;
    let mut targets = Vec::with_capacity(resolved_targets.len());
    let mut ineligible = Vec::new();
    for worker_id in resolved_targets {
        let worker = roster_worker(&roster, &worker_id).ok_or_else(|| {
            RuntimeStoreError::StateConflict(format!(
                "resolved Group target {worker_id} disappeared from its exact roster"
            ))
        })?;
        let introduction = HiveWorkerIntroductionStore::from_connection(tx)
            .get_by_worker(&worker_id)
            .map_err(RuntimeStoreError::Internal)?;
        if let Some(introduction) = introduction {
            if !introduction.status.allows_autonomy() {
                ineligible.push((worker.slug.clone(), introduction.status));
                continue;
            }
        }
        targets.push(worker_id);
    }
    if let Some((slug, status)) = ineligible.first() {
        if explicit_worker_targeting {
            return Err(RuntimeStoreError::StateConflict(format!(
                "@{slug} cannot join a Group turn while its Introduction is {status}; confirm or skip the Introduction first"
            )));
        }
    }
    if targets.is_empty() {
        return Err(RuntimeStoreError::StateConflict(
            "the group has no Worker whose Introduction is confirmed, skipped, or legacy-compatible"
                .into(),
        ));
    }

    // Append the trigger durably with the request's idempotency key. If a
    // previous execution of this exact request already appended it (for
    // example after an idempotency-record expiry), adopt that turn instead
    // of dispatching a duplicate fan-out.
    let turn_id = uuid::Uuid::new_v4().to_string();
    let trigger = hive_groups::append_message_with_conn(
        tx,
        &NewHiveGroupMessage {
            turn_id: Some(turn_id.clone()),
            idempotency_key: Some(idempotency_key.to_string()),
            ..NewHiveGroupMessage::user(&group.id, message)
        },
        now,
    )
    .map_err(RuntimeStoreError::Internal)?;
    if trigger.turn_id.as_deref() != Some(turn_id.as_str()) {
        return replayed_turn_response(tx, &group, &trigger.turn_id, &trigger.id, trigger.seq);
    }

    let policy = HiveGroupTurnPolicy::from(&group);
    let plan = build_speaker_plan(group.execution_mode, &targets, group.max_rounds);
    let turn = HiveGroupTurn {
        id: turn_id.clone(),
        group_id: group.id.clone(),
        trigger_message_id: trigger.id.clone(),
        execution_mode: group.execution_mode,
        policy,
        speaker_plan: plan.clone(),
        next_speaker_index: 0,
        status: HiveGroupTurnStatus::Running,
        member_outcomes: None,
        started_at: now.to_string(),
        finished_at: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    hive_groups::insert_turn_with_conn(tx, &turn).map_err(RuntimeStoreError::Internal)?;

    let excerpt = bounded_excerpt(message);
    let mut outcomes = Map::new();
    let mut events = Vec::new();
    let mut dispatched = 0usize;
    let next_speaker_index: usize = match group.execution_mode {
        HiveGroupExecutionMode::Workbench => {
            for (slot, worker_id) in plan.iter().enumerate() {
                let worker = roster_worker(&roster, worker_id);
                match dispatch_member_run(
                    tx,
                    now,
                    &group,
                    &turn,
                    worker,
                    Some(slot as u32),
                    &excerpt,
                    origin,
                )? {
                    Ok((run_id, run_events)) => {
                        dispatched += 1;
                        events.extend(run_events);
                        outcomes.insert(worker_id.clone(), dispatched_outcome(&run_id));
                    }
                    Err(reason) => {
                        outcomes.insert(worker_id.clone(), failed_outcome(&reason));
                    }
                }
            }
            plan.len()
        }
        // Roundtable and direct dispatch one speaker; roundtable advancement
        // continues in the pump as each run reaches a terminal state.
        HiveGroupExecutionMode::Roundtable | HiveGroupExecutionMode::Direct => {
            let mut index = 0usize;
            while index < plan.len() {
                let worker_id = &plan[index];
                let worker = roster_worker(&roster, worker_id);
                index += 1;
                match dispatch_member_run(tx, now, &group, &turn, worker, None, &excerpt, origin)? {
                    Ok((run_id, run_events)) => {
                        dispatched += 1;
                        events.extend(run_events);
                        outcomes.insert(worker_id.clone(), dispatched_outcome(&run_id));
                        break;
                    }
                    Err(reason) => {
                        outcomes.insert(worker_id.clone(), failed_outcome(&reason));
                        if group.execution_mode == HiveGroupExecutionMode::Direct {
                            break;
                        }
                    }
                }
            }
            if group.execution_mode == HiveGroupExecutionMode::Direct {
                plan.len()
            } else {
                index
            }
        }
    };

    hive_groups::update_turn_progress_with_conn(tx, &turn_id, next_speaker_index as u32, now)
        .map_err(RuntimeStoreError::Internal)?;
    let outcomes_value = Value::Object(outcomes);
    if outcomes_value
        .as_object()
        .is_some_and(|entries| !entries.is_empty())
    {
        hive_groups::update_turn_member_outcomes_with_conn(tx, &turn_id, &outcomes_value, now)
            .map_err(RuntimeStoreError::Internal)?;
    }

    let mut status = HiveGroupTurnStatus::Running;
    if dispatched == 0 {
        // Nothing could start: fail the turn now with per-member reasons so
        // the room shows an honest immediate outcome.
        status = HiveGroupTurnStatus::Failed;
        hive_groups::finalize_turn_with_conn(
            tx,
            &turn_id,
            HiveGroupTurnStatus::Failed,
            Some(&outcomes_value),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        hive_groups::append_message_with_conn(
            tx,
            &NewHiveGroupMessage {
                turn_id: Some(turn_id.clone()),
                ..NewHiveGroupMessage::system(
                    &group.id,
                    "No member Worker could start this turn; see the turn record for per-member reasons.",
                )
            },
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
    }

    // Every targeted member has now observed the room up to the trigger.
    for worker_id in &targets {
        hive_groups::advance_member_cursor_with_conn(
            tx,
            &group.id,
            worker_id,
            Some(trigger.seq),
            None,
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
    }

    Ok(Mutation {
        response: ResponsePayload::GroupTurn(GroupTurnResponse {
            group_id: group.id.clone(),
            turn_id,
            message_id: trigger.id,
            message_seq: trigger.seq,
            status: status.as_str().to_string(),
            target_worker_ids: targets,
        }),
        resource_id: Some(group.id),
        events,
    })
}

fn group_message_has_explicit_worker_target(
    roster: &[HiveWorker],
    message: &str,
    mentions_override: Option<&[String]>,
) -> bool {
    if mentions_override.is_some() {
        return true;
    }
    let mention_roster = roster
        .iter()
        .map(|worker| GroupMentionTarget {
            worker_id: worker.id.clone(),
            slug: worker.slug.clone(),
            display_name: worker.display_name.clone(),
        })
        .collect::<Vec<_>>();
    let resolution = hive_groups::parse_group_mentions(message, &mention_roster);
    resolution.saw_mention && !resolution.mentions_all && !resolution.explicit_worker_ids.is_empty()
}

pub(super) fn group_stop(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    group_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let group = require_owned_group(tx, actor, group_id)?;
    let Some(turn) =
        hive_groups::load_active_turn(tx, &group.id).map_err(RuntimeStoreError::Internal)?
    else {
        return Ok(Mutation {
            response: ack("no active group turn"),
            resource_id: Some(group.id),
            events: Vec::new(),
        });
    };

    let member_runs = load_member_runs(tx, &turn.id)?;
    let mut events = Vec::new();
    let mut outcomes = turn
        .member_outcomes
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for run in &member_runs {
        let worker_key = run
            .worker_id
            .clone()
            .unwrap_or_else(|| format!("run:{}", run.id));
        match run.status.as_str() {
            // Not yet executing: cancel durably right here.
            "queued" | "leased" | "sleeping" | "retry_wait" | "awaiting_input"
            | "recovery_required" => {
                tx.execute(
                    "UPDATE hive_runs
                     SET status = 'cancelled', lease_owner = NULL, lease_token = NULL,
                         lease_epoch = NULL, lease_expires_at = NULL, heartbeat_at = NULL,
                         wake_at = NULL, last_stop_reason = 'group turn stopped by user',
                         finished_at = ?2, updated_at = ?2
                     WHERE id = ?1 AND status = ?3",
                    params![run.id, now, run.status],
                )?;
                if run.status == "leased" {
                    if let Some(lease_token) = run.lease_token.as_deref() {
                        tx.execute(
                            "UPDATE hive_run_attempts
                             SET finished_at = ?4, outcome = 'cancelled',
                                 stop_reason = 'group turn stopped by user'
                             WHERE run_id = ?1 AND attempt_no = ?2 AND lease_token = ?3
                               AND finished_at IS NULL",
                            params![run.id, run.attempt_count, lease_token, now],
                        )?;
                    }
                }
                let controller = require_controller(tx, &run.session_id)?;
                events.push(append_event(
                    tx,
                    &controller,
                    "run_cancelled",
                    Some(&run.id),
                    None,
                    Some(&format!(
                        "transition:{}:{}:cancelled",
                        run.id, run.attempt_count
                    )),
                    json!({"run_id": run.id, "reason": "group turn stopped by user"}),
                    now,
                )?);
                outcomes.insert(worker_key, json!({"status": "cancelled"}));
            }
            // Executing now: record the request durably; the pump delivers a
            // CancelRun control to the exact hosted execution and the run
            // finishes as cancelled through the normal fenced path.
            "running" => {
                let controller = require_controller(tx, &run.session_id)?;
                events.push(append_event(
                    tx,
                    &controller,
                    "cancellation_requested",
                    Some(&run.id),
                    None,
                    Some(&format!(
                        "cancel_requested:{}:{}",
                        run.id, run.attempt_count
                    )),
                    json!({"run_id": run.id, "attempt": run.attempt_count}),
                    now,
                )?);
                outcomes.insert(
                    worker_key,
                    json!({"status": "cancelling", "run_id": run.id}),
                );
            }
            terminal => {
                outcomes
                    .entry(worker_key)
                    .or_insert_with(|| json!({"status": terminal, "run_id": run.id}));
            }
        }
    }

    hive_groups::finalize_turn_with_conn(
        tx,
        &turn.id,
        HiveGroupTurnStatus::Cancelled,
        Some(&Value::Object(outcomes)),
        now,
    )
    .map_err(RuntimeStoreError::Internal)?;
    hive_groups::append_message_with_conn(
        tx,
        &NewHiveGroupMessage {
            turn_id: Some(turn.id),
            ..NewHiveGroupMessage::system(&group.id, "Turn stopped by the user.")
        },
        now,
    )
    .map_err(RuntimeStoreError::Internal)?;

    Ok(Mutation {
        response: ack("group turn cancelled"),
        resource_id: Some(group.id),
        events,
    })
}

/// Archive and stop share one immediate handler transaction. This prevents
/// the group from disappearing from active UI surfaces while its provider
/// work remains live, and prevents a roundtable continuation from entering
/// between the stop and archive mutations.
pub(super) fn group_archive(
    tx: &Transaction<'_>,
    actor: &Actor,
    now: &str,
    group_id: &str,
) -> Result<Mutation, RuntimeStoreError> {
    let group = require_owned_group(tx, actor, group_id)?;
    let mut mutation = group_stop(tx, actor, now, &group.id)?;
    if group.status != HiveGroupStatus::Archived {
        let changed = tx.execute(
            "UPDATE hive_groups
             SET status = 'archived', updated_at = ?2
             WHERE id = ?1 AND status <> 'archived'",
            params![group.id, now],
        )?;
        if changed != 1 {
            return Err(RuntimeStoreError::StateConflict(
                "group archive lost its exact active-group state".into(),
            ));
        }
    }
    mutation.response = ack("group archived");
    mutation.resource_id = Some(group.id);
    Ok(mutation)
}

/// Exact-owner group load: a group owned by someone else is indistinguishable
/// from a missing one.
pub(super) fn require_owned_group(
    conn: &rusqlite::Connection,
    actor: &Actor,
    group_id: &str,
) -> Result<HiveGroup, RuntimeStoreError> {
    if group_id.trim().is_empty() || group_id.len() > 256 || group_id.as_bytes().contains(&0) {
        return Err(RuntimeStoreError::Invalid(
            "group id is invalid or exceeds 256 bytes".into(),
        ));
    }
    let group = hive_groups::load_group(conn, group_id).map_err(RuntimeStoreError::Internal)?;
    let Some(group) = group else {
        return Err(RuntimeStoreError::Ownership);
    };
    if group.user_id != actor.user_id {
        return Err(RuntimeStoreError::Ownership);
    }
    Ok(group)
}

/// Resolve turn targets from an explicit override or from mentions, then
/// apply the execution-mode routing rules.
fn resolve_targets(
    group: &HiveGroup,
    roster: &[HiveWorker],
    message: &str,
    mentions_override: Option<&[String]>,
) -> Result<Vec<String>, RuntimeStoreError> {
    let mention_roster = roster
        .iter()
        .map(|worker| GroupMentionTarget {
            worker_id: worker.id.clone(),
            slug: worker.slug.clone(),
            display_name: worker.display_name.clone(),
        })
        .collect::<Vec<_>>();

    let (resolved, was_explicit) = match mentions_override {
        Some(overrides) => {
            if overrides.is_empty() {
                return Err(RuntimeStoreError::Invalid(
                    "mentions_override must name at least one member slug".into(),
                ));
            }
            let mut targets = Vec::new();
            for slug in overrides {
                let slug = slug.trim().trim_start_matches('@');
                let Some(worker) = roster.iter().find(|worker| worker.slug == slug) else {
                    return Err(RuntimeStoreError::Invalid(format!(
                        "'{slug}' is not a member of this group"
                    )));
                };
                if !targets.contains(&worker.id) {
                    targets.push(worker.id.clone());
                }
            }
            (targets, true)
        }
        None => {
            let resolution = hive_groups::parse_group_mentions(message, &mention_roster);
            let explicit = resolution.saw_mention
                && !resolution.mentions_all
                && !resolution.explicit_worker_ids.is_empty();
            (
                resolution
                    .resolve_targets(&mention_roster)
                    .map_err(|error| RuntimeStoreError::Invalid(error.to_string()))?,
                explicit,
            )
        }
    };
    if resolved.is_empty() {
        return Err(RuntimeStoreError::StateConflict(
            "the group has no members to target".into(),
        ));
    }

    match group.execution_mode {
        HiveGroupExecutionMode::Direct => {
            if was_explicit {
                if resolved.len() > 1 {
                    return Err(RuntimeStoreError::Invalid(
                        "direct groups accept a single mentioned Worker per turn".into(),
                    ));
                }
                Ok(resolved)
            } else {
                let assignee = group
                    .default_assignee_worker_id
                    .clone()
                    .filter(|assignee| roster.iter().any(|worker| worker.id == *assignee))
                    .ok_or_else(|| {
                        RuntimeStoreError::StateConflict(
                            "this direct group has no default assignee; mention a Worker or set one"
                                .into(),
                        )
                    })?;
                Ok(vec![assignee])
            }
        }
        HiveGroupExecutionMode::Workbench | HiveGroupExecutionMode::Roundtable => Ok(resolved),
    }
}

/// The ordered dispatch plan. Workbench and direct are a single round;
/// roundtable rotates the speaker order every round (Grok-style) and is
/// capped by `max_rounds`.
pub(super) fn build_speaker_plan(
    mode: HiveGroupExecutionMode,
    targets: &[String],
    max_rounds: u32,
) -> Vec<String> {
    match mode {
        HiveGroupExecutionMode::Workbench | HiveGroupExecutionMode::Direct => targets.to_vec(),
        HiveGroupExecutionMode::Roundtable => {
            let member_count = targets.len();
            if member_count == 0 {
                return Vec::new();
            }
            (0..max_rounds as usize)
                .flat_map(|round| {
                    let shift = round % member_count;
                    targets[shift..]
                        .iter()
                        .chain(targets[..shift].iter())
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect()
        }
    }
}

/// One member run in a turn, as read back from hive_runs.
pub(super) struct MemberRunRow {
    pub(super) id: String,
    pub(super) session_id: String,
    pub(super) worker_id: Option<String>,
    pub(super) status: String,
    pub(super) attempt_count: i64,
    pub(super) lease_token: Option<String>,
    pub(super) last_error: Option<String>,
    pub(super) last_stop_reason: Option<String>,
}

pub(super) fn load_member_runs(
    conn: &rusqlite::Connection,
    turn_id: &str,
) -> Result<Vec<MemberRunRow>, RuntimeStoreError> {
    let mut statement = conn.prepare(
        "SELECT id, session_id, worker_id, status, attempt_count, lease_token,
                last_error, last_stop_reason
         FROM hive_runs
         WHERE group_turn_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = statement
        .query_map([turn_id], |row| {
            Ok(MemberRunRow {
                id: row.get(0)?,
                session_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                worker_id: row.get(2)?,
                status: row.get(3)?,
                attempt_count: row.get(4)?,
                lease_token: row.get(5)?,
                last_error: row.get(6)?,
                last_stop_reason: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Queue one member run on the Worker's private lane for this group. Returns
/// `Ok(Err(reason))` for per-member dispatch failures so one Worker's missing
/// prerequisites never abort the sibling fan-out.
#[allow(clippy::type_complexity)]
pub(super) fn dispatch_member_run(
    tx: &Transaction<'_>,
    now: &str,
    group: &HiveGroup,
    turn: &HiveGroupTurn,
    worker: Option<&HiveWorker>,
    workbench_slot: Option<u32>,
    trigger_excerpt: &str,
    origin: WorkerRunOrigin,
) -> Result<Result<(String, Vec<PersistedEvent>), String>, RuntimeStoreError> {
    let Some(worker) = worker else {
        return Ok(Err("the Worker left the group".into()));
    };
    if worker.status == HiveWorkerStatus::Archived {
        return Ok(Err("the Worker is archived".into()));
    }
    if worker.status == HiveWorkerStatus::Paused {
        return Ok(Err("the Worker is paused".into()));
    }
    if let Some(introduction) = HiveWorkerIntroductionStore::from_connection(tx)
        .get_by_worker(&worker.id)
        .map_err(RuntimeStoreError::Internal)?
    {
        if !introduction.status.allows_autonomy() {
            return Ok(Err(format!(
                "the Worker's Introduction is {}; confirm or skip it before Group work",
                introduction.status
            )));
        }
    }
    let Some(dm_session_id) = worker.dm_session_id.as_deref() else {
        return Ok(Err(
            "the Worker has no DM lane yet; open its DM once to create it".into(),
        ));
    };
    let actor = Actor {
        user_id: group.user_id.clone(),
        client_kind: "hive-group-turn".into(),
    };
    let dm_session = match require_owned_session(tx, &actor, dm_session_id) {
        Ok(session) => session,
        Err(RuntimeStoreError::Ownership) => {
            return Ok(Err("the Worker's DM lane is not reachable".into()))
        }
        Err(error) => return Err(error),
    };
    // The Worker's frozen model identity is authoritative; the DM session is
    // the compatibility fallback for Workers created without one.
    let model = worker
        .model
        .clone()
        .or_else(|| dm_session.model.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(model) = model else {
        return Ok(Err(
            "the Worker has no frozen model; pick one in the Worker editor".into(),
        ));
    };
    // Worker and session model keys are the same serialized shape on
    // different types (core vs. protocol); normalize to JSON for the frozen
    // run config after checking id consistency.
    let (model_key_id, model_key, model_catalog_revision) = if worker.model.is_some() {
        (
            worker.model_key.as_ref().map(|key| key.model_id.clone()),
            worker
                .model_key
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
            worker.model_catalog_revision.clone(),
        )
    } else {
        (
            dm_session
                .model_key
                .as_ref()
                .map(|key| key.model_id.clone()),
            dm_session
                .model_key
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
            dm_session.model_catalog_revision.clone(),
        )
    };
    if model_key_id.as_deref().is_some_and(|id| id != model) {
        return Ok(Err(
            "the Worker's model does not match its frozen model key".into(),
        ));
    }

    let session = ensure_group_worker_lane(
        tx,
        now,
        group,
        worker,
        &actor,
        &dm_session,
        &model,
        model_key.as_ref(),
        model_catalog_revision.as_deref(),
    )?;
    let controller = get_or_create_controller(tx, &session, now)?;
    let controller_bound = tx.execute(
        "UPDATE hive_controllers
         SET worker_id = ?2, scope_key = ?3, updated_at = ?4
         WHERE id = ?1 AND session_id = ?5 AND user_id IS ?6
           AND (worker_id IS NULL OR worker_id = ?2)",
        params![
            controller.id,
            worker.id,
            format!("worker:{}:group:{}", worker.id, group.id),
            now,
            session.id,
            group.user_id,
        ],
    )?;
    if controller_bound != 1 {
        return Ok(Err(
            "the group lane controller belongs to another Worker".into()
        ));
    }
    let objective = member_objective(&group.title, trigger_excerpt);

    // Workbench slots bound concurrent members of one turn to the group's
    // parallelism cap through the claim loop's concurrency-key rule; the
    // sequential modes serialize the whole turn on one key.
    let concurrency_key = match workbench_slot {
        Some(slot) => format!(
            "hive-group:{}:slot:{}",
            turn.id,
            slot % turn.policy.parallelism.max(1)
        ),
        None => format!("hive-group:{}", turn.id),
    };
    let run_id = uuid::Uuid::new_v4().to_string();
    let execution_context = HiveRunExecutionContextV1::worker_conversation_neutral(
        worker.id.clone(),
        worker.revision,
        WorkerConversationLane::Group {
            group_id: group.id.clone(),
        },
    )
    .map_err(RuntimeStoreError::Internal)?;
    let governor_lane_key = execution_context
        .lane()
        .canonical_lane_key()
        .map_err(RuntimeStoreError::Internal)?;
    let config = json!({
        "model": model,
        "model_key": model_key,
        "model_catalog_revision": model_catalog_revision,
        "permission_mode": worker.permission_mode.as_str(),
        "retry": RetryPolicy::default(),
        "group": {
            "group_id": group.id,
            "group_turn_id": turn.id,
            "worker_id": worker.id,
            "trigger_message_id": turn.trigger_message_id,
            "max_member_messages_per_turn": turn.policy.max_member_messages_per_turn,
            "context_window_messages": turn.policy.context_window_messages,
        },
    });
    tx.execute(
        "INSERT INTO hive_runs (
            id, controller_id, session_id, schedule_id, occurrence_id, kind,
            objective, config_json, status, priority, concurrency_key,
            scheduled_for, available_at, wake_at, attempt_count, max_attempts,
            lease_owner, lease_token, lease_epoch, lease_expires_at, heartbeat_at,
            last_stop_reason, last_error, outcome_json, created_at, started_at,
            finished_at, updated_at, worker_id, group_id, group_turn_id,
            trigger_message_id, governor_origin, governor_lane_key,
            execution_context_json
         ) VALUES (
            ?1, ?2, ?3, NULL, NULL, 'group_turn',
            ?4, ?5, 'queued', 0, ?6,
            ?7, ?7, NULL, 0, ?8, NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, ?7, NULL, NULL, ?7, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15
         )",
        params![
            run_id,
            controller.id,
            session.id,
            objective,
            serde_json::to_string(&config)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
            concurrency_key,
            now,
            MEMBER_RUN_MAX_ATTEMPTS,
            worker.id,
            group.id,
            turn.id,
            turn.trigger_message_id,
            origin.as_str(),
            governor_lane_key,
            serde_json::to_string(&execution_context)
                .map_err(|error| RuntimeStoreError::Internal(error.into()))?,
        ],
    )?;
    let event = append_event(
        tx,
        &controller,
        "run_queued",
        Some(&run_id),
        None,
        Some(&format!("run:{run_id}:queued")),
        json!({
            "run_id": run_id,
            "kind": "group_turn",
            "group_id": group.id,
            "group_turn_id": turn.id,
        }),
        now,
    )?;
    Ok(Ok((run_id, vec![event])))
}

/// Materialize or adopt the hidden conversation lane for one `(group,
/// Worker)` pair. The outer group mutation already holds an immediate SQLite
/// transaction, so session creation, lane binding, objective insertion, and
/// run enqueue either commit together or disappear together.
///
/// The deterministic candidate id prevents retries from accumulating orphan
/// sessions. Existing lanes are retained when a Worker leaves and later
/// rejoins the room; only operational configuration is refreshed from the
/// direct DM. No direct messages are copied.
#[allow(clippy::too_many_arguments)]
fn ensure_group_worker_lane(
    tx: &Transaction<'_>,
    now: &str,
    group: &HiveGroup,
    worker: &HiveWorker,
    actor: &Actor,
    dm_session: &super::persistence::OwnedSession,
    model: &str,
    model_key: Option<&Value>,
    model_catalog_revision: Option<&str>,
) -> Result<super::persistence::OwnedSession, RuntimeStoreError> {
    let candidate_session_id = group_worker_lane_session_id(&group.id, &worker.id);
    let existing = load_group_worker_lane_with_conn(tx, &group.id, &worker.id)
        .map_err(RuntimeStoreError::Internal)?;
    let session_id = if let Some(lane) = existing {
        if lane.session_id != candidate_session_id {
            return Err(RuntimeStoreError::StateConflict(
                "the group Worker lane points at a non-canonical session".into(),
            ));
        }
        lane.session_id
    } else {
        let model_key_json = model_key
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
        let inserted = tx.execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, work_mode, model,
                 model_key_json, model_catalog_revision, working_dir,
                 project_dir, workspace_mode, session_type, user_id,
                 target_branch, permission_mode
             )
             SELECT ?1, ?2, ?3, ?3, source.work_mode, ?4,
                    ?5, ?6, source.working_dir, source.project_dir,
                    source.workspace_mode, 'hive', source.user_id,
                    source.target_branch, ?7
             FROM sessions source
             WHERE source.id = ?8
               AND ((?9 IS NULL AND source.user_id IS NULL) OR source.user_id = ?9)",
            params![
                candidate_session_id,
                group_worker_lane_title(group, worker),
                now,
                model,
                model_key_json,
                model_catalog_revision,
                worker.permission_mode.as_str(),
                dm_session.id,
                actor.user_id,
            ],
        )?;
        if inserted != 1 {
            return Err(RuntimeStoreError::Ownership);
        }
        let lane = upsert_group_worker_lane_with_conn(
            tx,
            &NewHiveGroupWorkerLane::new(group.id.clone(), worker.id.clone(), candidate_session_id),
            now,
        )
        .map_err(RuntimeStoreError::Internal)?;
        lane.session_id
    };

    // A Worker's model, permissions, or workspace can change after its group
    // lane was first created. Refresh those execution inputs without touching
    // the lane's isolated transcript or its stable identity.
    let model_key_json = model_key
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| RuntimeStoreError::Internal(error.into()))?;
    let updated = tx.execute(
        "UPDATE sessions
         SET title = ?2,
             updated_at = ?3,
             work_mode = (SELECT work_mode FROM sessions WHERE id = ?4),
             model = ?5,
             model_key_json = ?6,
             model_catalog_revision = ?7,
             working_dir = (SELECT working_dir FROM sessions WHERE id = ?4),
             project_dir = (SELECT project_dir FROM sessions WHERE id = ?4),
             workspace_mode = (SELECT workspace_mode FROM sessions WHERE id = ?4),
             target_branch = (SELECT target_branch FROM sessions WHERE id = ?4),
             permission_mode = ?8
         WHERE id = ?1
           AND session_type = 'hive'
           AND ((?9 IS NULL AND user_id IS NULL) OR user_id = ?9)",
        params![
            session_id,
            group_worker_lane_title(group, worker),
            now,
            dm_session.id,
            model,
            model_key_json,
            model_catalog_revision,
            worker.permission_mode.as_str(),
            actor.user_id,
        ],
    )?;
    if updated != 1 {
        return Err(RuntimeStoreError::Ownership);
    }
    require_owned_session(tx, actor, &session_id)
}

pub(super) fn group_worker_lane_session_id(group_id: &str, worker_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("mitsuro:hive-group-worker-lane:{group_id}:{worker_id}").as_bytes(),
    )
    .to_string()
}

fn group_worker_lane_title(group: &HiveGroup, worker: &HiveWorker) -> String {
    format!("{} in {}", worker.display_name, group.title)
}

fn roster_worker<'roster>(
    roster: &'roster [HiveWorker],
    worker_id: &str,
) -> Option<&'roster HiveWorker> {
    roster.iter().find(|worker| worker.id == worker_id)
}

fn member_objective(group_title: &str, trigger_excerpt: &str) -> String {
    format!(
        "[GROUP TURN] You were addressed in the group room \"{group_title}\". Triggering message:\n{trigger_excerpt}\n\nReview the [GROUP ROOM] context for the roster and recent timeline. Respond once with the contribution you want shown in the room. The server will post that final response to the group after its exact run fence commits; do not invoke tools or claim that you posted it yourself."
    )
}

fn bounded_excerpt(message: &str) -> String {
    if message.len() <= MAX_TRIGGER_EXCERPT_BYTES {
        return message.to_string();
    }
    let mut end = MAX_TRIGGER_EXCERPT_BYTES;
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn dispatched_outcome(run_id: &str) -> Value {
    json!({"status": "dispatched", "run_id": run_id})
}

fn failed_outcome(reason: &str) -> Value {
    json!({"status": "failed", "error": reason})
}

/// Map one member run row onto its turn-outcome summary.
pub(super) fn member_run_outcome(run: &MemberRunRow) -> Value {
    let status = match run.status.as_str() {
        "succeeded" => "succeeded",
        "failed" => "failed",
        "dead_letter" => "failed",
        "cancelled" => "cancelled",
        "queued" => "queued",
        "leased" | "running" => "working",
        "sleeping" => "sleeping",
        "retry_wait" => "retrying",
        "awaiting_input" => "awaiting_input",
        "recovery_required" => "recovery_required",
        other => other,
    };
    let mut outcome = Map::new();
    outcome.insert("status".into(), Value::String(status.to_string()));
    outcome.insert("run_id".into(), Value::String(run.id.clone()));
    if run.status == "dead_letter" {
        outcome.insert("dead_letter".into(), Value::Bool(true));
    }
    if let Some(error) = run
        .last_error
        .as_deref()
        .or(run.last_stop_reason.as_deref())
        .filter(|_| matches!(run.status.as_str(), "failed" | "dead_letter"))
    {
        outcome.insert("error".into(), Value::String(bounded_excerpt(error)));
    }
    Value::Object(outcome)
}

/// Aggregate classification of a finished turn from its member outcome map.
/// Any success keeps the turn at least partial: one failed provider yields a
/// partial outcome, never a destroyed room.
pub(super) fn classify_turn_outcomes(outcomes: &Value) -> HiveGroupTurnStatus {
    let Some(entries) = outcomes.as_object() else {
        return HiveGroupTurnStatus::Failed;
    };
    if entries.is_empty() {
        return HiveGroupTurnStatus::Failed;
    }
    let statuses = entries
        .values()
        .map(|entry| {
            entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("failed")
        })
        .collect::<Vec<_>>();
    let succeeded = statuses
        .iter()
        .filter(|status| **status == "succeeded")
        .count();
    let cancelled = statuses
        .iter()
        .filter(|status| **status == "cancelled")
        .count();
    if succeeded == statuses.len() {
        HiveGroupTurnStatus::Completed
    } else if succeeded > 0 {
        HiveGroupTurnStatus::Partial
    } else if cancelled == statuses.len() {
        HiveGroupTurnStatus::Cancelled
    } else {
        HiveGroupTurnStatus::Failed
    }
}

/// A replayed trigger append means an earlier execution of this exact
/// request already created the turn; report that turn instead of forking.
fn replayed_turn_response(
    tx: &Transaction<'_>,
    group: &HiveGroup,
    existing_turn_id: &Option<String>,
    message_id: &str,
    message_seq: i64,
) -> Result<Mutation, RuntimeStoreError> {
    let turn = existing_turn_id
        .as_deref()
        .map(|turn_id| hive_groups::load_turn(tx, turn_id))
        .transpose()
        .map_err(RuntimeStoreError::Internal)?
        .flatten();
    let Some(turn) = turn else {
        return Err(RuntimeStoreError::Conflict(
            "this message was already delivered with a different turn record".into(),
        ));
    };
    Ok(Mutation {
        response: ResponsePayload::GroupTurn(GroupTurnResponse {
            group_id: group.id.clone(),
            turn_id: turn.id,
            message_id: message_id.to_string(),
            message_seq,
            status: turn.status.as_str().to_string(),
            target_worker_ids: turn.speaker_plan,
        }),
        resource_id: Some(group.id.clone()),
        events: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use mitsuro_core::storage::hive_groups::{
        self, HiveGroupExecutionMode, HiveGroupStore, HiveGroupTurnStatus, NewHiveGroup,
    };
    use mitsuro_core::storage::{
        Database, HiveGroupStatus, HiveWorkerStore, NewHiveWorker, SessionManager, SessionType,
        WorkerRunOrigin, WorkspaceMode,
    };
    use mitsuro_hive_protocol::{Actor, GroupMessageCommand, ResponsePayload};
    use rusqlite::{params, Transaction, TransactionBehavior};
    use tempfile::TempDir;

    use super::super::persistence::RuntimeStoreError;
    use super::{build_speaker_plan, classify_turn_outcomes, group_message, group_stop};

    struct GroupWorld {
        db_path: std::path::PathBuf,
        group_id: String,
        workers: Vec<(String, String, String)>, // (worker_id, slug, dm_session_id)
        _temp: TempDir,
    }

    fn seed_world(mode: HiveGroupExecutionMode, slugs: &[&str], caps: GroupCaps) -> GroupWorld {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("groups-engine.db");
        let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
        let worker_store = HiveWorkerStore::new(Database::new(&db_path).unwrap());
        let mut workers = Vec::new();
        for slug in slugs {
            let dm_session_id = session_manager
                .create_session_for_user_with_config(
                    &format!("{slug} DM"),
                    Some("test:model"),
                    None,
                    None,
                    WorkspaceMode::Neutral,
                    None,
                    None,
                    SessionType::Hive,
                )
                .unwrap();
            let worker = worker_store
                .create(&NewHiveWorker {
                    model: Some("test:model".into()),
                    dm_session_id: Some(dm_session_id.clone()),
                    ..NewHiveWorker::new(*slug)
                })
                .unwrap();
            workers.push((worker.id, slug.to_string(), dm_session_id));
        }
        let group_store = HiveGroupStore::new(Database::new(&db_path).unwrap());
        let group = group_store
            .create(&NewHiveGroup {
                user_id: None,
                title: "Release Room".into(),
                execution_mode: mode,
                max_rounds: Some(caps.max_rounds),
                max_member_messages_per_turn: Some(2),
                parallelism: Some(caps.parallelism),
                context_window_messages: Some(24),
                default_assignee_worker_id: caps
                    .assignee_index
                    .map(|index| workers[index].0.clone()),
                member_worker_ids: workers.iter().map(|(id, _, _)| id.clone()).collect(),
            })
            .unwrap();
        GroupWorld {
            db_path,
            group_id: group.id,
            workers,
            _temp: temp,
        }
    }

    struct GroupCaps {
        max_rounds: u32,
        parallelism: u32,
        assignee_index: Option<usize>,
    }

    impl Default for GroupCaps {
        fn default() -> Self {
            Self {
                max_rounds: 3,
                parallelism: 3,
                assignee_index: None,
            }
        }
    }

    fn send(
        world: &GroupWorld,
        actor: Actor,
        message: &str,
        idempotency_key: &str,
    ) -> Result<ResponsePayload, RuntimeStoreError> {
        send_to_group(world, &world.group_id, actor, message, idempotency_key)
    }

    fn send_to_group(
        world: &GroupWorld,
        group_id: &str,
        actor: Actor,
        message: &str,
        idempotency_key: &str,
    ) -> Result<ResponsePayload, RuntimeStoreError> {
        let db = Database::new(&world.db_path).unwrap();
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let result = group_message(
            &tx,
            &actor,
            &now,
            GroupMessageCommand {
                group_id: group_id.to_string(),
                message: message.to_string(),
                mentions_override: None,
            },
            idempotency_key,
            WorkerRunOrigin::UserGroup,
        );
        tx.commit().unwrap();
        result.map(|mutation| mutation.response)
    }

    fn turn_response(response: ResponsePayload) -> mitsuro_hive_protocol::GroupTurnResponse {
        match response {
            ResponsePayload::GroupTurn(turn) => turn,
            other => panic!("expected group turn response, got {other:?}"),
        }
    }

    struct RunRow {
        session_id: String,
        worker_id: Option<String>,
        status: String,
        concurrency_key: Option<String>,
        kind: String,
        objective: String,
        trigger_message_id: Option<String>,
    }

    fn member_runs(world: &GroupWorld, turn_id: &str) -> Vec<RunRow> {
        let db = Database::new(&world.db_path).unwrap();
        let mut statement = db
            .conn()
            .prepare(
                "SELECT session_id, worker_id, status, concurrency_key, kind,
                        objective, trigger_message_id
                 FROM hive_runs WHERE group_turn_id = ?1 ORDER BY created_at ASC, id ASC",
            )
            .unwrap();
        let rows = statement
            .query_map([turn_id], |row| {
                Ok(RunRow {
                    session_id: row.get(0)?,
                    worker_id: row.get(1)?,
                    status: row.get(2)?,
                    concurrency_key: row.get(3)?,
                    kind: row.get(4)?,
                    objective: row.get(5)?,
                    trigger_message_id: row.get(6)?,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    }

    fn set_introduction_status(world: &GroupWorld, worker_index: usize, status: &str) {
        let db = Database::new(&world.db_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO hive_worker_introductions (
                     worker_id, run_id, status, prompt_version, created_at, updated_at,
                     completed_at
                 ) VALUES (?1, NULL, ?2, 1, ?3, ?3,
                           CASE WHEN ?2 IN ('confirmed', 'skipped') THEN ?3 ELSE NULL END)",
                params![world.workers[worker_index].0, status, now],
            )
            .unwrap();
    }

    #[test]
    fn workbench_fan_out_queues_every_target_with_bounded_slots() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher", "reviewer", "builder"],
            GroupCaps {
                parallelism: 2,
                ..GroupCaps::default()
            },
        );
        let turn = turn_response(
            send(
                &world,
                Actor::local("test"),
                "kick off the review",
                "turn-1",
            )
            .unwrap(),
        );
        assert_eq!(turn.status, "running");
        assert_eq!(turn.target_worker_ids.len(), 3);

        let runs = member_runs(&world, &turn.turn_id);
        assert_eq!(runs.len(), 3, "workbench queues every target immediately");
        assert!(runs.iter().all(|run| run.status == "queued"));
        assert!(runs.iter().all(|run| run.kind == "group_turn"));
        // Slot keys bound concurrent members to the parallelism cap through
        // the claim loop's concurrency-key rule.
        let slots = runs
            .iter()
            .filter_map(|run| run.concurrency_key.clone())
            .collect::<HashSet<_>>();
        assert_eq!(slots.len(), 2, "3 members share 2 parallelism slots");

        // Each member gets a private group lane. The direct DM remains clean,
        // and each run is queued on the lane recorded for its exact pair.
        let db = Database::new(&world.db_path).unwrap();
        for (worker_id, _, dm_session_id) in &world.workers {
            let dm_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM messages
                     WHERE session_id = ?1 AND role = 'user' AND content LIKE '%GROUP TURN%'",
                    [dm_session_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(dm_count, 0, "group turns must never enter the direct DM");
            let lane_session_id: String = db
                .conn()
                .query_row(
                    "SELECT session_id FROM hive_group_worker_lanes
                     WHERE group_id = ?1 AND worker_id = ?2",
                    params![world.group_id, worker_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                lane_session_id,
                super::group_worker_lane_session_id(&world.group_id, worker_id),
                "the same group and Worker must always resolve to the same lane"
            );
            assert_ne!(lane_session_id, *dm_session_id);
            let run = runs
                .iter()
                .find(|run| run.worker_id.as_deref() == Some(worker_id.as_str()))
                .expect("every target must have an exact member run");
            assert_eq!(run.session_id, lane_session_id);
            assert!(run.objective.contains("[GROUP TURN]"));
            assert!(run.objective.contains("kick off the review"));
            assert_eq!(
                run.trigger_message_id.as_deref(),
                Some(turn.message_id.as_str())
            );
            let lane_count: i64 = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM messages
                     WHERE session_id = ?1 AND role = 'user' AND content LIKE '%GROUP TURN%'",
                    [&lane_session_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                lane_count, 0,
                "the room trigger is structural context, not a shadow user message"
            );
        }

        // Replaying the exact request reuses the appended trigger and turn.
        let replay = turn_response(
            send(
                &world,
                Actor::local("test"),
                "kick off the review",
                "turn-1",
            )
            .unwrap(),
        );
        assert_eq!(replay.turn_id, turn.turn_id);
        assert_eq!(replay.message_seq, turn.message_seq);
        assert_eq!(member_runs(&world, &turn.turn_id).len(), 3);
    }

    #[test]
    fn explicit_group_mention_rejects_every_unfinished_introduction_state() {
        for status in [
            "queued",
            "running",
            "awaiting_context",
            "review_ready",
            "failed",
            "needs_recovery",
        ] {
            let world = seed_world(
                HiveGroupExecutionMode::Workbench,
                &["setup-worker"],
                GroupCaps::default(),
            );
            set_introduction_status(&world, 0, status);
            let result = send(
                &world,
                Actor::local("test"),
                "@setup-worker inspect the release",
                &format!("introduction-{status}"),
            );
            assert!(matches!(
                result,
                Err(RuntimeStoreError::StateConflict(message))
                    if message.contains("@setup-worker")
                        && message.contains("confirm or skip")
                        && message.contains(status)
            ));
            let db = Database::new(&world.db_path).unwrap();
            let (messages, turns, runs): (i64, i64, i64) = db
                .conn()
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM hive_group_messages),
                         (SELECT COUNT(*) FROM hive_group_turns),
                         (SELECT COUNT(*) FROM hive_runs)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!((messages, turns, runs), (0, 0, 0));
        }
    }

    #[test]
    fn untargeted_group_turn_skips_unintroduced_members_without_spending_on_them() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["still-setting-up", "legacy-ready"],
            GroupCaps::default(),
        );
        set_introduction_status(&world, 0, "awaiting_context");
        let turn = turn_response(
            send(
                &world,
                Actor::local("test"),
                "inspect the release",
                "skip-unintroduced-member",
            )
            .unwrap(),
        );
        assert_eq!(turn.target_worker_ids, vec![world.workers[1].0.clone()]);
        let runs = member_runs(&world, &turn.turn_id);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].worker_id.as_deref(),
            Some(world.workers[1].0.as_str())
        );
    }

    #[test]
    fn confirmed_skipped_and_legacy_workers_remain_group_eligible() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["confirmed-worker", "skipped-worker", "legacy-worker"],
            GroupCaps::default(),
        );
        set_introduction_status(&world, 0, "confirmed");
        set_introduction_status(&world, 1, "skipped");
        let turn = turn_response(
            send(
                &world,
                Actor::local("test"),
                "inspect the release",
                "eligible-introductions",
            )
            .unwrap(),
        );
        assert_eq!(turn.target_worker_ids.len(), 3);
        assert_eq!(member_runs(&world, &turn.turn_id).len(), 3);
    }

    #[test]
    fn direct_and_group_a_and_b_transcripts_are_isolated_and_readded_members_reuse_lane() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher", "reviewer"],
            GroupCaps::default(),
        );
        let worker_id = world.workers[0].0.clone();
        let dm_session_id = world.workers[0].2.clone();
        SessionManager::new(Database::new(&world.db_path).unwrap())
            .save_message(
                &dm_session_id,
                "user",
                r#"[{"type":"text","text":"DM-CANARY-ONLY"}]"#,
            )
            .unwrap();

        let group_store = HiveGroupStore::new(Database::new(&world.db_path).unwrap());
        let group_b = group_store
            .create(&NewHiveGroup {
                title: "Second Room".into(),
                execution_mode: HiveGroupExecutionMode::Workbench,
                member_worker_ids: vec![worker_id.clone()],
                ..NewHiveGroup::default()
            })
            .unwrap();

        let group_a_turn = turn_response(
            send(
                &world,
                Actor::local("test"),
                "GROUP-A-CANARY",
                "isolation-a-1",
            )
            .unwrap(),
        );
        let group_b_turn = turn_response(
            send_to_group(
                &world,
                &group_b.id,
                Actor::local("test"),
                "GROUP-B-CANARY",
                "isolation-b-1",
            )
            .unwrap(),
        );

        let db = Database::new(&world.db_path).unwrap();
        let lane_for = |group_id: &str| -> String {
            db.conn()
                .query_row(
                    "SELECT session_id FROM hive_group_worker_lanes
                     WHERE group_id = ?1 AND worker_id = ?2",
                    params![group_id, worker_id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        let lane_a = lane_for(&world.group_id);
        let lane_b = lane_for(&group_b.id);
        assert_ne!(lane_a, lane_b);
        assert_ne!(lane_a, dm_session_id);
        assert_ne!(lane_b, dm_session_id);
        let group_a_run = member_runs(&world, &group_a_turn.turn_id)
            .into_iter()
            .find(|run| run.worker_id.as_deref() == Some(worker_id.as_str()))
            .unwrap();
        assert_eq!(group_a_run.session_id, lane_a);
        assert_eq!(
            group_a_run.trigger_message_id.as_deref(),
            Some(group_a_turn.message_id.as_str())
        );
        assert!(group_a_run.objective.contains("GROUP-A-CANARY"));
        let group_b_run = member_runs(&world, &group_b_turn.turn_id)
            .into_iter()
            .find(|run| run.worker_id.as_deref() == Some(worker_id.as_str()))
            .unwrap();
        assert_eq!(group_b_run.session_id, lane_b);
        assert_eq!(
            group_b_run.trigger_message_id.as_deref(),
            Some(group_b_turn.message_id.as_str())
        );
        assert!(group_b_run.objective.contains("GROUP-B-CANARY"));

        let session_transcript = |session_id: &str| -> String {
            let mut statement = db
                .conn()
                .prepare("SELECT content FROM messages WHERE session_id = ?1 ORDER BY id")
                .unwrap();
            let rows = statement
                .query_map([session_id], |row| row.get::<_, String>(0))
                .unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join("\n")
        };
        let room_transcript = |group_id: &str| -> String {
            let mut statement = db
                .conn()
                .prepare(
                    "SELECT content FROM hive_group_messages
                     WHERE group_id = ?1 ORDER BY seq ASC",
                )
                .unwrap();
            let rows = statement
                .query_map([group_id], |row| row.get::<_, String>(0))
                .unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join("\n")
        };
        let dm = session_transcript(&dm_session_id);
        let lane_a_transcript = session_transcript(&lane_a);
        let lane_b_transcript = session_transcript(&lane_b);
        let group_a = room_transcript(&world.group_id);
        let group_b_transcript = room_transcript(&group_b.id);
        assert!(dm.contains("DM-CANARY-ONLY"));
        assert!(!dm.contains("GROUP-A-CANARY"));
        assert!(!dm.contains("GROUP-B-CANARY"));
        assert!(lane_a_transcript.is_empty());
        assert!(lane_b_transcript.is_empty());
        assert!(group_a.contains("GROUP-A-CANARY"));
        assert!(!group_a.contains("DM-CANARY-ONLY"));
        assert!(!group_a.contains("GROUP-B-CANARY"));
        assert!(group_b_transcript.contains("GROUP-B-CANARY"));
        assert!(!group_b_transcript.contains("DM-CANARY-ONLY"));
        assert!(!group_b_transcript.contains("GROUP-A-CANARY"));

        // Removing and re-adding a member does not erase or replace its lane.
        group_store
            .set_members(&world.group_id, &[world.workers[1].0.clone()])
            .unwrap();
        group_store
            .set_members(
                &world.group_id,
                &[worker_id.clone(), world.workers[1].0.clone()],
            )
            .unwrap();
        let second_a = turn_response(
            send(
                &world,
                Actor::local("test"),
                "GROUP-A-AFTER-READD",
                "isolation-a-2",
            )
            .unwrap(),
        );
        let second_a_run = member_runs(&world, &second_a.turn_id)
            .into_iter()
            .find(|run| run.worker_id.as_deref() == Some(worker_id.as_str()))
            .unwrap();
        assert_eq!(second_a_run.session_id, lane_a);
        assert_eq!(lane_for(&world.group_id), lane_a);
    }

    #[test]
    fn roundtable_queues_one_speaker_and_plans_rotated_rounds() {
        let world = seed_world(
            HiveGroupExecutionMode::Roundtable,
            &["alpha", "beta", "gamma"],
            GroupCaps {
                max_rounds: 2,
                ..GroupCaps::default()
            },
        );
        let turn = turn_response(
            send(&world, Actor::local("test"), "table discussion", "turn-rt").unwrap(),
        );
        let runs = member_runs(&world, &turn.turn_id);
        assert_eq!(runs.len(), 1, "roundtable queues only the first speaker");
        assert_eq!(
            runs[0].worker_id.as_deref(),
            Some(world.workers[0].0.as_str())
        );
        assert_eq!(
            runs[0].concurrency_key.as_deref(),
            Some(format!("hive-group:{}", turn.turn_id).as_str()),
            "the whole roundtable serializes on one key"
        );

        let db = Database::new(&world.db_path).unwrap();
        let stored = hive_groups::load_turn(db.conn(), &turn.turn_id)
            .unwrap()
            .unwrap();
        let ids = |indexes: &[usize]| {
            indexes
                .iter()
                .map(|index| world.workers[*index].0.clone())
                .collect::<Vec<_>>()
        };
        // Two rounds over [a, b, c]: round two rotates the speaker order.
        assert_eq!(stored.speaker_plan, ids(&[0, 1, 2, 1, 2, 0]));
        assert_eq!(stored.next_speaker_index, 1);
        assert_eq!(stored.policy.max_rounds, 2);
    }

    #[test]
    fn direct_routes_to_the_assignee_or_the_single_mention() {
        let world = seed_world(
            HiveGroupExecutionMode::Direct,
            &["planner", "builder"],
            GroupCaps {
                assignee_index: Some(1),
                ..GroupCaps::default()
            },
        );
        // No mention: the default assignee handles the turn.
        let assigned =
            turn_response(send(&world, Actor::local("test"), "handle this", "direct-1").unwrap());
        assert_eq!(assigned.target_worker_ids, vec![world.workers[1].0.clone()]);

        // A single mention overrides the assignee.
        let mentioned = turn_response(
            send(
                &world,
                Actor::local("test"),
                "@planner handle this",
                "direct-2",
            )
            .unwrap(),
        );
        assert_eq!(
            mentioned.target_worker_ids,
            vec![world.workers[0].0.clone()]
        );

        // Mentioning several members is invalid in direct mode.
        let error = send(
            &world,
            Actor::local("test"),
            "@planner @builder both of you",
            "direct-3",
        )
        .unwrap_err();
        assert!(matches!(error, RuntimeStoreError::Invalid(_)), "{error:?}");
    }

    #[test]
    fn invalid_mentions_fail_before_persisting_or_fanning_out() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher", "reviewer"],
            GroupCaps::default(),
        );

        let ambiguous = send(
            &world,
            Actor::local("test"),
            "@re compare the evidence",
            "ambiguous-mention",
        )
        .unwrap_err();
        let ambiguous = match ambiguous {
            RuntimeStoreError::Invalid(message) => message,
            other => panic!("expected invalid mention error, got {other:?}"),
        };
        assert!(ambiguous.contains("@researcher"), "{ambiguous}");
        assert!(ambiguous.contains("@reviewer"), "{ambiguous}");

        let unresolved = send(
            &world,
            Actor::local("test"),
            "@nobody compare the evidence",
            "unknown-mention",
        )
        .unwrap_err();
        let unresolved = match unresolved {
            RuntimeStoreError::Invalid(message) => message,
            other => panic!("expected invalid mention error, got {other:?}"),
        };
        assert!(unresolved.contains("@nobody"), "{unresolved}");

        let db = Database::new(&world.db_path).unwrap();
        let persisted: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_group_messages WHERE group_id = ?1",
                [&world.group_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, 0, "invalid routing must be side-effect free");
        let runs: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE kind = 'group_turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(runs, 0, "invalid routing must not enqueue Worker runs");
    }

    #[test]
    fn dispatch_failures_isolate_siblings_and_all_failures_fail_the_turn() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["healthy", "modelless"],
            GroupCaps::default(),
        );
        // Strip the second worker's frozen model everywhere so its dispatch
        // fails while the sibling proceeds.
        let db = Database::new(&world.db_path).unwrap();
        db.conn()
            .execute(
                "UPDATE hive_workers SET model = NULL, model_key_json = NULL WHERE id = ?1",
                [&world.workers[1].0],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE sessions SET model = NULL WHERE id = ?1",
                [&world.workers[1].2],
            )
            .unwrap();

        let turn =
            turn_response(send(&world, Actor::local("test"), "split work", "isolate-1").unwrap());
        assert_eq!(
            turn.status, "running",
            "one healthy member keeps the turn alive"
        );
        let runs = member_runs(&world, &turn.turn_id);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].worker_id.as_deref(),
            Some(world.workers[0].0.as_str())
        );

        let stored = hive_groups::load_turn(db.conn(), &turn.turn_id)
            .unwrap()
            .unwrap();
        let outcomes = stored.member_outcomes.unwrap();
        assert_eq!(outcomes[&world.workers[1].0]["status"], "failed");
        assert!(outcomes[&world.workers[1].0]["error"]
            .as_str()
            .unwrap()
            .contains("model"));

        // When nobody can start, the turn fails immediately with reasons.
        db.conn()
            .execute(
                "UPDATE hive_workers SET model = NULL, model_key_json = NULL",
                [],
            )
            .unwrap();
        db.conn()
            .execute("UPDATE sessions SET model = NULL", [])
            .unwrap();
        let failed = turn_response(
            send(
                &world,
                Actor::local("test"),
                "split work again",
                "isolate-2",
            )
            .unwrap(),
        );
        assert_eq!(failed.status, "failed");
        let failed_turn = hive_groups::load_turn(db.conn(), &failed.turn_id)
            .unwrap()
            .unwrap();
        assert_eq!(failed_turn.status, HiveGroupTurnStatus::Failed);
        assert!(failed_turn.finished_at.is_some());
    }

    #[test]
    fn group_commands_are_exact_owner_scoped() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher"],
            GroupCaps::default(),
        );
        let bob = Actor {
            user_id: Some("bob".into()),
            client_kind: "test".into(),
        };
        let error = send(&world, bob.clone(), "hello", "owner-1").unwrap_err();
        assert!(matches!(error, RuntimeStoreError::Ownership), "{error:?}");

        let db = Database::new(&world.db_path).unwrap();
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        let stop_error =
            group_stop(&tx, &bob, &chrono::Utc::now().to_rfc3339(), &world.group_id).unwrap_err();
        assert!(matches!(stop_error, RuntimeStoreError::Ownership));
    }

    #[test]
    fn group_stop_cancels_queued_members_and_marks_the_turn_cancelled() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher", "builder"],
            GroupCaps::default(),
        );
        let turn =
            turn_response(send(&world, Actor::local("test"), "start please", "stop-1").unwrap());

        let db = Database::new(&world.db_path).unwrap();
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        let mutation = group_stop(
            &tx,
            &Actor::local("test"),
            &chrono::Utc::now().to_rfc3339(),
            &world.group_id,
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(matches!(mutation.response, ResponsePayload::Ack(ack) if ack.accepted));

        let runs = member_runs(&world, &turn.turn_id);
        assert!(runs.iter().all(|run| run.status == "cancelled"));
        let stored = hive_groups::load_turn(db.conn(), &turn.turn_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, HiveGroupTurnStatus::Cancelled);
        assert!(stored.finished_at.is_some());
        // The room shows the stop as a system message on the same turn.
        let store = HiveGroupStore::new(Database::new(&world.db_path).unwrap());
        let messages = store.list_messages_after(&world.group_id, 0, 10).unwrap();
        assert!(messages.iter().any(|message| {
            message.turn_id.as_deref() == Some(turn.turn_id.as_str())
                && message.content.contains("stopped")
        }));

        // Stopping an idle group acknowledges without effect.
        let tx = Transaction::new_unchecked(db.conn(), TransactionBehavior::Immediate).unwrap();
        let idle = group_stop(
            &tx,
            &Actor::local("test"),
            &chrono::Utc::now().to_rfc3339(),
            &world.group_id,
        )
        .unwrap();
        assert!(
            matches!(idle.response, ResponsePayload::Ack(ack) if ack.message.as_deref() == Some("no active group turn"))
        );
    }

    #[test]
    fn archived_groups_reject_new_turns() {
        let world = seed_world(
            HiveGroupExecutionMode::Workbench,
            &["researcher"],
            GroupCaps::default(),
        );
        HiveGroupStore::new(Database::new(&world.db_path).unwrap())
            .set_status(&world.group_id, HiveGroupStatus::Archived)
            .unwrap();
        let error = send(&world, Actor::local("test"), "hello", "archived-1").unwrap_err();
        assert!(matches!(error, RuntimeStoreError::StateConflict(_)));
    }

    #[test]
    fn speaker_plans_follow_mode_semantics() {
        let targets = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            build_speaker_plan(HiveGroupExecutionMode::Workbench, &targets, 5),
            targets
        );
        assert_eq!(
            build_speaker_plan(HiveGroupExecutionMode::Direct, &targets[..1], 5),
            vec!["a".to_string()]
        );
        assert_eq!(
            build_speaker_plan(HiveGroupExecutionMode::Roundtable, &targets, 3),
            vec!["a", "b", "c", "b", "c", "a", "c", "a", "b"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn turn_outcome_classification_table() {
        let classify = |entries: &[(&str, &str)]| {
            let map = entries
                .iter()
                .map(|(worker, status)| (worker.to_string(), serde_json::json!({"status": status})))
                .collect::<serde_json::Map<_, _>>();
            classify_turn_outcomes(&serde_json::Value::Object(map))
        };
        assert_eq!(
            classify(&[("a", "succeeded"), ("b", "succeeded")]),
            HiveGroupTurnStatus::Completed
        );
        assert_eq!(
            classify(&[("a", "succeeded"), ("b", "failed")]),
            HiveGroupTurnStatus::Partial
        );
        assert_eq!(
            classify(&[("a", "succeeded"), ("b", "cancelled")]),
            HiveGroupTurnStatus::Partial
        );
        assert_eq!(
            classify(&[("a", "failed"), ("b", "failed")]),
            HiveGroupTurnStatus::Failed
        );
        assert_eq!(
            classify(&[("a", "cancelled"), ("b", "cancelled")]),
            HiveGroupTurnStatus::Cancelled
        );
        assert_eq!(
            classify(&[("a", "failed"), ("b", "cancelled")]),
            HiveGroupTurnStatus::Failed
        );
        assert_eq!(classify(&[]), HiveGroupTurnStatus::Failed);
    }
}
