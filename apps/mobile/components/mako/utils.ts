import type {
  AutonomousTask,
  MakoCurrentRunSummary,
  MakoHomeStatus,
  MakoRunWakeEvent,
  MakoSessionStatus,
} from "@krusty/api";

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

export function getRunDisplayStatus(
  run: Pick<MakoCurrentRunSummary, "runtime" | "agent_state">,
): MakoHomeStatus {
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

export function describeRun(summary: MakoCurrentRunSummary): string {
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
  return parts.join(" • ") || "No tasks yet";
}

export function getRunGroup(summary: MakoCurrentRunSummary): "waiting" | "active" | "sleeping" | "queued" | "completed" {
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

function taskWakeEvent(task: AutonomousTask, completedIds: Set<string>): MakoRunWakeEvent {
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

export function buildWakeEvents(status: MakoSessionStatus | null): MakoRunWakeEvent[] {
  if (!status) {
    return [];
  }

  const wake: MakoRunWakeEvent[] = [];

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
