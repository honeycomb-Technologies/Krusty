use krusty_core::storage::{MakoRunPriority, MakoRuntimeStateStatus};

use super::{MakoCurrentRunSummary, MakoPendingApprovalSummary};

pub(super) fn compare_pending_approvals(
    left: &MakoPendingApprovalSummary,
    right: &MakoPendingApprovalSummary,
) -> std::cmp::Ordering {
    let priority_order = priority_rank(right.priority).cmp(&priority_rank(left.priority));
    if priority_order != std::cmp::Ordering::Equal {
        return priority_order;
    }

    let requested_order = left.requested_at.cmp(&right.requested_at);
    if requested_order != std::cmp::Ordering::Equal {
        return requested_order;
    }

    left.session_title
        .cmp(&right.session_title)
        .then_with(|| left.tool_name.cmp(&right.tool_name))
}

pub(super) fn compare_run_summaries(
    left: &MakoCurrentRunSummary,
    right: &MakoCurrentRunSummary,
) -> std::cmp::Ordering {
    let left_priority = left
        .runtime
        .as_ref()
        .map(|runtime| runtime.priority)
        .unwrap_or(MakoRunPriority::Normal);
    let right_priority = right
        .runtime
        .as_ref()
        .map(|runtime| runtime.priority)
        .unwrap_or(MakoRunPriority::Normal);
    let priority_order = priority_rank(right_priority).cmp(&priority_rank(left_priority));
    if priority_order != std::cmp::Ordering::Equal {
        return priority_order;
    }

    let left_scheduled = left
        .runtime
        .as_ref()
        .map(|runtime| {
            runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled")
        })
        .unwrap_or(false);
    let right_scheduled = right
        .runtime
        .as_ref()
        .map(|runtime| {
            runtime.status == MakoRuntimeStateStatus::Sleeping
                && runtime.sleep_reason.as_deref() == Some("scheduled")
        })
        .unwrap_or(false);

    if left_scheduled && right_scheduled {
        let wake_order = left
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.next_wake_at.as_ref())
            .cmp(
                &right
                    .runtime
                    .as_ref()
                    .and_then(|runtime| runtime.next_wake_at.as_ref()),
            );
        if wake_order != std::cmp::Ordering::Equal {
            return wake_order;
        }
    }

    right.updated_at.cmp(&left.updated_at)
}

fn priority_rank(priority: MakoRunPriority) -> u8 {
    match priority {
        MakoRunPriority::High => 2,
        MakoRunPriority::Normal => 1,
        MakoRunPriority::Low => 0,
    }
}
