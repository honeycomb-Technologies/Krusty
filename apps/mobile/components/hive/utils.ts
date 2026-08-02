import type {
  AutonomousTask,
  HiveCurrentRunSummary,
  HiveHomeStatus,
  HiveRunPriority,
  HiveRunWakeEvent,
  HiveSessionStatus,
} from "@mitsuro/api";
import { formatPriorityLabel } from "./priority";

const ACTIVE_STALE_MS = 30 * 60 * 1000;
const WAITING_STALE_MS = 15 * 60 * 1000;
const QUEUED_STALE_MS = 60 * 60 * 1000;
const OVERDUE_WAKE_GRACE_MS = 5 * 60 * 1000;

function completedTaskIds(tasks: AutonomousTask[]): Set<string> {
  return new Set(
    tasks
      .filter((task) => task.status === "completed")
      .map((task) => task.id),
  );
}

export function formatRelativeTime(value?: string | null): string {
  if (!value) {
    return "No activity";
  }

  const diff = Date.now() - new Date(value).getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h ago`;
  }
  const days = Math.floor(hours / 24);
  if (days < 7) {
    return `${days}d ago`;
  }
  return new Date(value).toLocaleDateString();
}

export function formatTimestamp(value?: string | null): string {
  if (!value) {
    return "Pending";
  }
  return new Date(value).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function formatProjectLabel(path?: string | null): string {
  if (!path) {
    return "No project selected";
  }

  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) {
    return path;
  }
  return parts.slice(-2).join("/");
}

export function getRunPriority(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): HiveRunPriority {
  return run.runtime?.priority ?? "normal";
}

export function isScheduledRun(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): boolean {
  return (
    run.runtime?.status === "sleeping" &&
    run.runtime.sleep_reason === "scheduled"
  );
}

export function isFailedRun(
  run: Pick<HiveCurrentRunSummary, "runtime" | "agent_state">,
): boolean {
  return run.runtime?.status === "error" || run.agent_state === "error";
}

export function getRunNextWakeAt(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): string | null {
  if (isScheduledRun(run) || run.runtime?.status === "sleeping") {
    return run.runtime?.next_wake_at ?? null;
  }

  return null;
}

export function formatRunMeta(summary: HiveCurrentRunSummary): string {
  const parts = [formatProjectLabel(summary.project_dir)];
  if (summary.runtime?.crew_slug) {
    parts.push(`agent ${summary.runtime.crew_slug}`);
  }
  const priority = getRunPriority(summary);
  if (priority !== "normal") {
    parts.push(formatPriorityLabel(priority));
  }
  parts.push(formatRelativeTime(summary.updated_at));
  return parts.join(" • ");
}

export function getRunDisplayStatus(
  run: Pick<HiveCurrentRunSummary, "runtime" | "agent_state">,
): HiveHomeStatus {
  switch (run.runtime?.status) {
    case "running":
      return "awake";
    case "sleeping":
      return "sleeping";
    case "paused":
      return "paused";
    case "awaiting_input":
    case "error":
      return "blocked";
    default:
      break;
  }

  switch (run.agent_state) {
    case "streaming":
    case "tool_executing":
      return "awake";
    case "awaiting_input":
    case "error":
      return "blocked";
    default:
      return "idle";
  }
}

export function canPauseRun(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): boolean {
  return (
    run.runtime?.status !== "sleeping" && run.runtime?.status !== "paused"
  );
}

export function canResumeRun(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): boolean {
  return run.runtime?.status !== "running";
}

export function getRunResumeLabel(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): string {
  return run.runtime?.status === "sleeping" ? "Wake now" : "Resume";
}

export function getRuntimeLabel(status: string): string {
  switch (status) {
    case "running":
    case "awake":
      return "awake";
    case "awaiting_input":
      return "blocked";
    case "cancelled":
      return "cancelled";
    case "error":
      return "failed";
    default:
      return status;
  }
}

export function describeRun(summary: HiveCurrentRunSummary): string {
  const parts: string[] = [];
  if (summary.in_progress_tasks > 0) {
    parts.push(`${summary.in_progress_tasks} active`);
  }
  if (summary.pending_tasks > 0) {
    parts.push(`${summary.pending_tasks} pending`);
  }
  if (summary.blocked_tasks > 0) {
    parts.push(`${summary.blocked_tasks} blocked`);
  }
  if (summary.completed_tasks > 0) {
    parts.push(`${summary.completed_tasks} done`);
  }
  if (summary.failed_tasks > 0) {
    parts.push(`${summary.failed_tasks} failed`);
  }
  if (
    parts.length === 0 &&
    summary.runtime?.status === "sleeping" &&
    summary.runtime.sleep_reason === "scheduled"
  ) {
    return `Scheduled for ${formatTimestamp(summary.runtime.next_wake_at)}`;
  }
  return parts.join(" • ") || "No tasks yet";
}

export function getRunGroup(summary: HiveCurrentRunSummary): "waiting" | "active" | "sleeping" | "queued" | "completed" {
  if (isScheduledRun(summary)) {
    return "queued";
  }
  const displayStatus = getRunDisplayStatus(summary);
  if (displayStatus === "blocked") {
    return "waiting";
  }
  if (displayStatus === "awake" || displayStatus === "paused") {
    return "active";
  }
  if (displayStatus === "sleeping") {
    return "sleeping";
  }
  const openTasks = summary.pending_tasks + summary.in_progress_tasks + summary.blocked_tasks;
  if (openTasks > 0) {
    return "queued";
  }
  return "completed";
}

export function getQueueHeadRuns(
  runs: HiveCurrentRunSummary[],
): HiveCurrentRunSummary[] {
  return runs.filter((run) => getRunGroup(run) !== "completed");
}

export function getAttentionRuns(
  runs: HiveCurrentRunSummary[],
): HiveCurrentRunSummary[] {
  return runs.filter((run) => {
    const kind = run.diagnostic?.kind;
    if (kind) {
      return kind === "awaiting_approval" || kind === "awaiting_input" || kind === "failed";
    }
    return getRunGroup(run) === "waiting" || isFailedRun(run);
  });
}

export function isOverdueScheduledRun(
  run: Pick<HiveCurrentRunSummary, "runtime">,
): boolean {
  const wakeAt = run.runtime?.next_wake_at;
  if (!isScheduledRun(run) || !wakeAt) {
    return false;
  }

  return Date.now() - new Date(wakeAt).getTime() > OVERDUE_WAKE_GRACE_MS;
}

export function isStaleRun(run: HiveCurrentRunSummary): boolean {
  if (isFailedRun(run) || isOverdueScheduledRun(run)) {
    return true;
  }

  const updatedAt = run.updated_at;
  if (!updatedAt) {
    return false;
  }

  const ageMs = Date.now() - new Date(updatedAt).getTime();
  switch (getRunGroup(run)) {
    case "active":
      return ageMs > ACTIVE_STALE_MS;
    case "waiting":
      return ageMs > WAITING_STALE_MS;
    case "queued":
      return ageMs > QUEUED_STALE_MS;
    default:
      return false;
  }
}

export function getStaleRuns(
  runs: HiveCurrentRunSummary[],
): HiveCurrentRunSummary[] {
  return runs.filter((run) => {
    const kind = run.diagnostic?.kind;
    if (kind) {
      return (
        kind === "overdue_wake" ||
        kind === "stale_active" ||
        kind === "stale_waiting" ||
        kind === "stale_queued"
      );
    }
    return isStaleRun(run);
  });
}

export function describeRunDrift(run: HiveCurrentRunSummary): string {
  if (run.diagnostic) {
    return run.diagnostic.detail;
  }
  if (isOverdueScheduledRun(run)) {
    return "Wake is overdue";
  }
  if (isFailedRun(run)) {
    return "Run failed and needs review";
  }

  const age = formatRelativeTime(run.updated_at);
  switch (getRunGroup(run)) {
    case "active":
      return `No new activity for ${age}`;
    case "waiting":
      return `Blocked for ${age}`;
    case "queued":
      return `Queued without movement for ${age}`;
    default:
      return `Last activity ${age}`;
  }
}

function taskWakeEvent(task: AutonomousTask, completedIds: Set<string>): HiveRunWakeEvent {
  const blocked =
    task.status === "pending" &&
    task.blocked_by.some((dependency) => !completedIds.has(dependency));
  const status = blocked ? "blocked" : task.status;
  let detail = task.description || task.result || null;

  if (!detail && task.owner) {
    detail = `Owned by ${task.owner}`;
  } else if (task.owner && task.status === "in_progress") {
    detail = `${detail} • ${task.owner}`;
  }

  return {
    id: `task-${task.id}`,
    timestamp: task.updated_at,
    title: task.subject,
    detail,
    kind: "task",
    status,
  };
}

export function buildWakeEvents(status: HiveSessionStatus | null): HiveRunWakeEvent[] {
  if (!status) {
    return [];
  }

  const wake: HiveRunWakeEvent[] = [];

  if (status.runtime) {
    const runtime = status.runtime;
    const detail =
      runtime.sleep_reason ??
      runtime.last_error ??
      runtime.last_wake_reason ??
      null;

    wake.push({
      id: `runtime-${runtime.updated_at}`,
      timestamp: runtime.updated_at,
      title: `Runtime ${getRuntimeLabel(runtime.status)}`,
      detail,
      kind: "runtime",
      status: getRuntimeLabel(runtime.status),
    });
  }

  const completedIds = completedTaskIds(status.tasks);
  wake.push(...status.tasks.map((task) => taskWakeEvent(task, completedIds)));

  return wake.sort((left, right) => {
    return new Date(right.timestamp).getTime() - new Date(left.timestamp).getTime();
  });
}
