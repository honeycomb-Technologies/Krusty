import type { MakoRunPriority } from "@krusty/api";

export const MAKO_PRIORITY_OPTIONS: Array<{
  id: MakoRunPriority;
  label: string;
}> = [
  { id: "low", label: "Low" },
  { id: "normal", label: "Normal" },
  { id: "high", label: "High" },
];

export function describePriority(priority: MakoRunPriority): string {
  switch (priority) {
    case "high":
      return "High-priority runs float to the top across Current, Runs, and Status.";
    case "low":
      return "Low-priority runs stay available, but they yield visual priority to more urgent work.";
    case "normal":
      return "Normal priority keeps this run in the standard queue order.";
  }
}

export function formatPriorityLabel(priority: MakoRunPriority): string {
  switch (priority) {
    case "high":
      return "High priority";
    case "low":
      return "Low priority";
    case "normal":
      return "Normal priority";
  }
}
