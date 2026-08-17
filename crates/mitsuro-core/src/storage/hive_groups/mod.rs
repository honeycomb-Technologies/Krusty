//! Group rooms where Hive Workers collaborate.
//!
//! A group references Workers (never owns them), carries the execution policy
//! for group turns (workbench/roundtable/direct plus caps), and stores an
//! append-only, per-group-sequenced message timeline. `hive_group_turns` is
//! the durable aggregate of one user-triggered turn: it snapshots the policy,
//! records the speaker plan, and accumulates per-member outcomes so one failed
//! provider yields a partial turn instead of a destroyed room.

mod mentions;
mod model;
mod store;

#[cfg(test)]
mod tests;

pub use mentions::{parse_group_mentions, GroupMentionTarget, MentionResolution};
pub use model::{
    HiveGroup, HiveGroupExecutionMode, HiveGroupMember, HiveGroupMessage, HiveGroupRunContext,
    HiveGroupSenderKind, HiveGroupStatus, HiveGroupTurn, HiveGroupTurnPolicy, HiveGroupTurnStatus,
    HiveGroupUpdate, HiveMemberCursor, NewHiveGroup, NewHiveGroupMessage,
    MAX_HIVE_GROUP_MESSAGE_BYTES,
};
pub use store::{
    advance_member_cursor_with_conn, append_message_with_conn, finalize_turn_with_conn,
    insert_turn_with_conn, latest_seq_with_conn, load_active_turn, load_group, load_member_workers,
    load_recent_messages, load_turn, update_turn_member_outcomes_with_conn,
    update_turn_progress_with_conn, CappedGroupAppend, HiveGroupStore,
};
