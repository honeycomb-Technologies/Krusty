export type MakoSchedulePreset = "now" | "30m" | "2h" | "tomorrow";

interface MakoSchedulePresetOption {
  id: MakoSchedulePreset;
  label: string;
}

const ALL_PRESETS: MakoSchedulePresetOption[] = [
  { id: "now", label: "Now" },
  { id: "30m", label: "30m" },
  { id: "2h", label: "2h" },
  { id: "tomorrow", label: "Tomorrow" },
];

export function schedulePresetOptions(
  includeImmediate: boolean,
): MakoSchedulePresetOption[] {
  if (includeImmediate) {
    return ALL_PRESETS;
  }

  return ALL_PRESETS.filter((preset) => preset.id !== "now");
}

export function resolveScheduleStartAt(
  preset: MakoSchedulePreset,
): string | null {
  const now = new Date();
  switch (preset) {
    case "now":
      return null;
    case "30m":
      return new Date(now.getTime() + 30 * 60 * 1000).toISOString();
    case "2h":
      return new Date(now.getTime() + 2 * 60 * 60 * 1000).toISOString();
    case "tomorrow": {
      const tomorrow = new Date(now);
      tomorrow.setDate(tomorrow.getDate() + 1);
      tomorrow.setHours(9, 0, 0, 0);
      return tomorrow.toISOString();
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
        ? "New work opens as a run inside Mako."
        : "This run is active now.";
    case "30m":
      return subject === "course"
        ? "Mako will queue this run and wake it in 30 minutes."
        : "Mako will wake this run in 30 minutes.";
    case "2h":
      return subject === "course"
        ? "Mako will queue this run and wake it in two hours."
        : "Mako will wake this run in two hours.";
    case "tomorrow":
      return subject === "course"
        ? "Mako will queue this run for tomorrow morning."
        : "Mako will wake this run tomorrow morning.";
  }
}
