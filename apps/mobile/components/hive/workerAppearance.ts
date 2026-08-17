import type { HiveWorker } from "@mitsuro/api";

/** Preset avatar palette for Hive Workers. */
export const HIVE_WORKER_COLORS = [
  "#E8B04B",
  "#7743DB",
  "#3E9C6B",
  "#D96A4B",
  "#4B87D9",
  "#C24BB0",
];

/** Stable fallback color for Workers created without an explicit color. */
export function workerFallbackColor(slug: string): string {
  let hash = 0;
  for (const char of slug) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return HIVE_WORKER_COLORS[hash % HIVE_WORKER_COLORS.length];
}

export function workerAvatarColor(worker: HiveWorker): string {
  return worker.avatar_color ?? workerFallbackColor(worker.slug);
}

export function workerInitials(displayName: string): string {
  const words = displayName.trim().split(/\s+/).filter(Boolean);
  const initials = words
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase() ?? "")
    .join("");
  return initials || "W";
}

export function workerAutonomyLabel(worker: HiveWorker): string {
  switch (worker.autonomy) {
    case "always_on":
      return "Always on";
    case "scheduled":
      return "Scheduled";
    default:
      return "Manual";
  }
}

/** One-line roster meta: pinned model (or default) plus autonomy. */
export function workerMetaLine(worker: HiveWorker): string {
  const provider = worker.model_key?.provider;
  const model = worker.model
    ? provider
      ? `${worker.model} · ${provider}`
      : worker.model
    : "Default model";
  return `${model} · ${workerAutonomyLabel(worker)}`;
}
