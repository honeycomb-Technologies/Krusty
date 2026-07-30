export type MakoSchedulePreset = "now" | "30m" | "2h" | "tomorrow" | "custom";

export interface MakoScheduleResolution {
  startAt: string | null;
  error: string | null;
}

interface MakoSchedulePresetOption {
  id: MakoSchedulePreset;
  label: string;
}

const ALL_PRESETS: MakoSchedulePresetOption[] = [
  { id: "now", label: "Now" },
  { id: "30m", label: "30m" },
  { id: "2h", label: "2h" },
  { id: "tomorrow", label: "Tomorrow" },
  { id: "custom", label: "Custom" },
];

export function schedulePresetOptions(
  includeImmediate: boolean,
): MakoSchedulePresetOption[] {
  if (includeImmediate) {
    return ALL_PRESETS;
  }

  return ALL_PRESETS.filter((preset) => preset.id !== "now");
}

export function formatScheduleInputValue(value?: string | null): string {
  if (!value) {
    return "";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return "";
  }

  const pad = (segment: number) => String(segment).padStart(2, "0");
  return `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())} ${pad(parsed.getHours())}:${pad(parsed.getMinutes())}`;
}

function normalizeCustomScheduleInput(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return "";
  }

  const normalized = trimmed.includes("T")
    ? trimmed
    : trimmed.replace(/\s+/, "T");
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(normalized)) {
    return `${normalized}:00`;
  }

  return normalized;
}

export function resolveScheduleSelection(
  preset: MakoSchedulePreset,
  customValue?: string | null,
): MakoScheduleResolution {
  const now = new Date();
  switch (preset) {
    case "now":
      return { startAt: null, error: null };
    case "30m":
      return {
        startAt: new Date(now.getTime() + 30 * 60 * 1000).toISOString(),
        error: null,
      };
    case "2h":
      return {
        startAt: new Date(now.getTime() + 2 * 60 * 60 * 1000).toISOString(),
        error: null,
      };
    case "tomorrow": {
      const tomorrow = new Date(now);
      tomorrow.setDate(tomorrow.getDate() + 1);
      tomorrow.setHours(9, 0, 0, 0);
      return { startAt: tomorrow.toISOString(), error: null };
    }
    case "custom": {
      const normalized = normalizeCustomScheduleInput(customValue ?? "");
      if (!normalized) {
        return {
          startAt: null,
          error: "Enter a wake time like 2026-04-08 09:30.",
        };
      }

      const parsed = new Date(normalized);
      if (Number.isNaN(parsed.getTime())) {
        return {
          startAt: null,
          error: "Enter a valid local date and time.",
        };
      }

      if (parsed.getTime() <= now.getTime()) {
        return {
          startAt: null,
          error: "Wake time must be in the future.",
        };
      }

      return {
        startAt: parsed.toISOString(),
        error: null,
      };
    }
  }
}

export function describeSchedulePreset(
  preset: MakoSchedulePreset,
  subject: "course" | "run",
): string {
  switch (preset) {
    case "now":
      return subject === "course"
        ? "New work opens as a run inside Hive."
        : "This run is active now.";
    case "30m":
      return subject === "course"
        ? "Hive will queue this run and wake it in 30 minutes."
        : "Hive will wake this run in 30 minutes.";
    case "2h":
      return subject === "course"
        ? "Hive will queue this run and wake it in two hours."
        : "Hive will wake this run in two hours.";
    case "tomorrow":
      return subject === "course"
        ? "Hive will queue this run for tomorrow morning."
        : "Hive will wake this run tomorrow morning.";
    case "custom":
      return subject === "course"
        ? "Choose a specific local wake time for this new run."
        : "Choose a specific local wake time for this run.";
  }
}
