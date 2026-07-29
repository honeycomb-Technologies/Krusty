import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Modal,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  type DimensionValue,
  Text,
  TextInput,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { ListRowsSkeleton } from "../ui/Skeleton";
import type { MakoCurrentRunSummary, MakoGlobalSchedule } from "@krusty/api";
import type { MakoCurrentState } from "./types";
import {
  formatScheduleInputValue,
  resolveScheduleSelection,
} from "./schedule";
import { formatProjectLabel, getRunNextWakeAt } from "./utils";

interface MakoScheduleViewProps {
  state: MakoCurrentState;
  onSelectRun: (runId: string) => void;
  onOpenProject?: (projectDir: string, targetBranch?: string | null) => Promise<void> | void;
}

type ScheduleMode = "month_day" | "week" | "month";

interface ScheduledRunItem {
  run: MakoCurrentRunSummary;
  wakeAt: string;
  scheduledAt: Date;
  dayKey: string;
  timeValue: string;
  title: string;
  detail: string;
  seriesKey: string;
}

interface WeekLaneBar {
  id: string;
  item: ScheduledRunItem;
  startIndex: number;
  span: number;
  dayCount: number;
}

interface ScheduleDetailTarget {
  item: ScheduledRunItem;
  patternDays: number[];
  spanLabel: string | null;
}

const WEEKDAY_LABELS = ["S", "M", "T", "W", "T", "F", "S"] as const;
const WEEKDAY_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

function dayKeyFromDate(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function dayKey(value: string): string {
  return dayKeyFromDate(new Date(value));
}

function timeValue(value: string): string {
  const parsed = new Date(value);
  const hour = `${parsed.getHours()}`.padStart(2, "0");
  const minute = `${parsed.getMinutes()}`.padStart(2, "0");
  return `${hour}:${minute}`;
}

function timeLabel(value: string): string {
  return new Date(value).toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
}

function recurrenceLabel(schedule: MakoGlobalSchedule): string {
  const recurrence = schedule.recurrence;
  switch (recurrence.kind) {
    case "once":
      return "One time";
    case "daily":
      return `Daily at ${recurrence.time}`;
    case "weekdays":
      return `Weekdays at ${recurrence.time}`;
    case "weekly":
      return `${recurrence.weekdays.join(", ")} at ${recurrence.time}`;
    case "monthly":
      return `Monthly on day ${recurrence.day} at ${recurrence.time}`;
  }
}

function nextFireLabel(schedule: MakoGlobalSchedule): string {
  if (!schedule.next_fire_at) {
    return schedule.status === "paused" ? "Paused" : "No next wake";
  }
  return new Date(schedule.next_fire_at).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function dayLabel(date: Date): string {
  return date.toLocaleDateString([], {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

function monthStart(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), 1);
}

function shiftMonth(date: Date, delta: number): Date {
  return new Date(date.getFullYear(), date.getMonth() + delta, 1);
}

function sameDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

function addDays(date: Date, delta: number): Date {
  const next = new Date(date);
  next.setDate(date.getDate() + delta);
  return next;
}

function startOfWeek(date: Date): Date {
  return addDays(date, -date.getDay());
}

function diffInDays(left: Date, right: Date): number {
  const leftOnly = new Date(left.getFullYear(), left.getMonth(), left.getDate()).getTime();
  const rightOnly = new Date(
    right.getFullYear(),
    right.getMonth(),
    right.getDate(),
  ).getTime();
  return Math.round((leftOnly - rightOnly) / (24 * 60 * 60 * 1000));
}

function buildCalendarDays(baseMonth: Date): Date[] {
  const start = monthStart(baseMonth);
  const offset = start.getDay();
  const gridStart = addDays(start, -offset);

  return Array.from({ length: 42 }, (_, index) => addDays(gridStart, index));
}

function scheduleSeriesKey(run: MakoCurrentRunSummary, wakeAt: string): string {
  const title = (run.title || "Untitled run").trim().toLowerCase();
  const project = run.project_dir?.trim().toLowerCase() ?? "";
  const branch = run.target_branch?.trim().toLowerCase() ?? "";
  const crew = run.runtime?.crew_slug?.trim().toLowerCase() ?? "";
  return [title, project, branch, crew, timeValue(wakeAt)].join("|");
}

function buildScheduledItems(runs: MakoCurrentRunSummary[]): ScheduledRunItem[] {
  return runs
    .flatMap((run) => {
      const wakeAt = getRunNextWakeAt(run);
      if (!wakeAt) {
        return [];
      }

      const detailParts: string[] = [];
      if (run.project_dir) {
        detailParts.push(formatProjectLabel(run.project_dir));
      }
      if (run.target_branch) {
        detailParts.push(`branch ${run.target_branch}`);
      }
      if (run.runtime?.crew_slug) {
        detailParts.push(`${run.runtime.crew_slug} crew`);
      }

      return [
        {
          run,
          wakeAt,
          scheduledAt: new Date(wakeAt),
          dayKey: dayKey(wakeAt),
          timeValue: timeValue(wakeAt),
          title: run.title || "Untitled run",
          detail: detailParts.join(" • ") || "Scheduled by Hive",
          seriesKey: scheduleSeriesKey(run, wakeAt),
        },
      ];
    })
    .sort((left, right) => left.wakeAt.localeCompare(right.wakeAt));
}

function buildSeriesDayMap(items: ScheduledRunItem[]): Map<string, number[]> {
  const map = new Map<string, Set<number>>();
  for (const item of items) {
    const bucket = map.get(item.seriesKey) ?? new Set<number>();
    bucket.add(item.scheduledAt.getDay());
    map.set(item.seriesKey, bucket);
  }

  return new Map(
    [...map.entries()].map(([key, days]) => [
      key,
      [...days].sort((left, right) => left - right),
    ]),
  );
}

function formatPatternLabel(days: number[]): string | null {
  if (days.length <= 1) {
    return null;
  }
  if (days.length === 7) {
    return "daily";
  }
  if (days.join(",") === "1,2,3,4,5") {
    return "weekdays";
  }
  return days.map((day) => WEEKDAY_SHORT[day]).join(" ");
}

function buildWeekLanes(items: ScheduledRunItem[], weekStartDate: Date): WeekLaneBar[] {
  const weekEnd = addDays(weekStartDate, 7);
  const grouped = new Map<string, ScheduledRunItem[]>();

  for (const item of items) {
    if (item.scheduledAt < weekStartDate || item.scheduledAt >= weekEnd) {
      continue;
    }
    const bucket = grouped.get(item.seriesKey) ?? [];
    bucket.push(item);
    grouped.set(item.seriesKey, bucket);
  }

  const bars: WeekLaneBar[] = [];
  for (const bucket of grouped.values()) {
    const sorted = [...bucket].sort((left, right) => left.wakeAt.localeCompare(right.wakeAt));
    let segmentStart = sorted[0];
    let previousIndex = diffInDays(sorted[0].scheduledAt, weekStartDate);
    let count = 1;

    for (let index = 1; index < sorted.length; index += 1) {
      const current = sorted[index];
      const currentIndex = diffInDays(current.scheduledAt, weekStartDate);
      if (currentIndex === previousIndex + 1) {
        previousIndex = currentIndex;
        count += 1;
        continue;
      }

      bars.push({
        id: `${segmentStart.run.session_id}:${diffInDays(segmentStart.scheduledAt, weekStartDate)}`,
        item: segmentStart,
        startIndex: diffInDays(segmentStart.scheduledAt, weekStartDate),
        span: count,
        dayCount: count,
      });
      segmentStart = current;
      previousIndex = currentIndex;
      count = 1;
    }

    bars.push({
      id: `${segmentStart.run.session_id}:${diffInDays(segmentStart.scheduledAt, weekStartDate)}`,
      item: segmentStart,
      startIndex: diffInDays(segmentStart.scheduledAt, weekStartDate),
      span: count,
      dayCount: count,
    });
  }

  return bars.sort((left, right) => {
    if (left.startIndex !== right.startIndex) {
      return left.startIndex - right.startIndex;
    }
    if (left.span !== right.span) {
      return right.span - left.span;
    }
    return left.item.wakeAt.localeCompare(right.item.wakeAt);
  });
}

function packWeekLanes(bars: WeekLaneBar[]): WeekLaneBar[][] {
  const lanes: WeekLaneBar[][] = [];

  for (const bar of bars) {
    let placed = false;
    for (const lane of lanes) {
      const overlaps = lane.some((candidate) => {
        const candidateEnd = candidate.startIndex + candidate.span;
        const currentEnd = bar.startIndex + bar.span;
        return bar.startIndex < candidateEnd && currentEnd > candidate.startIndex;
      });
      if (!overlaps) {
        lane.push(bar);
        placed = true;
        break;
      }
    }
    if (!placed) {
      lanes.push([bar]);
    }
  }

  return lanes;
}

function monthMarkerColor(count: number, colors: ReturnType<typeof useThemeContext>["theme"]["colors"]) {
  if (count > 2) {
    return colors.userMessage;
  }
  return colors.mutedForeground;
}

function ScheduleModeToggle({
  mode,
  onChange,
}: {
  mode: ScheduleMode;
  onChange: (mode: ScheduleMode) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  const items: Array<{ id: ScheduleMode; label: string }> = [
    { id: "month_day", label: "Month + day" },
    { id: "week", label: "Week" },
    { id: "month", label: "Month" },
  ];

  return (
    <View style={[styles.toggleRow, { borderColor: t.border }]}>
      {items.map((item) => {
        const active = item.id === mode;
        return (
          <Pressable
            key={item.id}
            onPress={() => {
              if (active) {
                return;
              }
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              onChange(item.id);
            }}
            style={[
              styles.toggleButton,
              {
                backgroundColor: active ? t.card : "transparent",
              },
            ]}
          >
            <Text
              style={[
                styles.toggleLabel,
                { color: active ? t.foreground : t.mutedForeground },
              ]}
            >
              {item.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function ScheduleRow({
  item,
  patternLabel,
  onPress,
}: {
  item: ScheduledRunItem;
  patternLabel: string | null;
  onPress: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <Pressable
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onPress();
      }}
      style={[styles.row, { borderColor: t.border }]}
    >
      <View style={styles.rowCopy}>
        <Text style={[styles.rowTitle, { color: t.foreground }]} numberOfLines={1}>
          {item.title}
        </Text>
        <Text style={[styles.rowDetail, { color: t.mutedForeground }]} numberOfLines={1}>
          {item.detail}
        </Text>
        <View style={styles.metaRow}>
          <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
            {timeLabel(item.wakeAt)}
          </Text>
          {patternLabel ? (
            <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
              {patternLabel}
            </Text>
          ) : null}
        </View>
      </View>
      <Text style={[styles.rowAction, { color: t.userMessage }]}>Details</Text>
    </Pressable>
  );
}

function ScheduleDetailSheet({
  visible,
  target,
  titleValue,
  wakeInput,
  onChangeTitle,
  onChangeWakeInput,
  onSave,
  onClose,
  onWakeNow,
  onOpenRun,
  onOpenProject,
  isSaving,
  error,
}: {
  visible: boolean;
  target: ScheduleDetailTarget | null;
  titleValue: string;
  wakeInput: string;
  onChangeTitle: (value: string) => void;
  onChangeWakeInput: (value: string) => void;
  onSave: () => void;
  onClose: () => void;
  onWakeNow: () => void;
  onOpenRun: () => void;
  onOpenProject: () => void;
  isSaving: boolean;
  error: string | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (!target) {
    return null;
  }

  return (
    <Modal visible={visible} animationType="fade" transparent onRequestClose={onClose}>
      <View style={styles.sheetOverlay}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
        <View
          style={[
            styles.sheet,
            {
              backgroundColor: t.background,
              borderColor: t.border,
            },
          ]}
        >
          <View style={styles.sheetHeader}>
            <View style={styles.sheetCopy}>
              <Text style={[styles.sheetTitle, { color: t.foreground }]}>Schedule item</Text>
              <Text style={[styles.sheetSubtitle, { color: t.mutedForeground }]}>
                Minimal timing and naming controls
              </Text>
            </View>
            <Pressable onPress={onClose} style={styles.sheetClose}>
              <Text style={[styles.sheetCloseLabel, { color: t.mutedForeground }]}>Close</Text>
            </Pressable>
          </View>

          <View style={styles.fieldBlock}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Title</Text>
            <TextInput
              value={titleValue}
              onChangeText={onChangeTitle}
              placeholder="Schedule title"
              placeholderTextColor={`${t.mutedForeground}aa`}
              style={[
                styles.fieldInput,
                {
                  color: t.foreground,
                  backgroundColor: t.card,
                  borderColor: t.border,
                },
              ]}
            />
          </View>

          <View style={styles.fieldBlock}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Detail</Text>
            <Text style={[styles.readonlyValue, { color: t.foreground }]}>
              {target.item.detail}
            </Text>
          </View>

          <View style={styles.fieldBlock}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Time</Text>
            <TextInput
              value={wakeInput}
              onChangeText={onChangeWakeInput}
              placeholder="2026-04-11 09:30"
              placeholderTextColor={`${t.mutedForeground}aa`}
              autoCapitalize="none"
              autoCorrect={false}
              style={[
                styles.fieldInput,
                {
                  color: t.foreground,
                  backgroundColor: t.card,
                  borderColor: t.border,
                },
              ]}
            />
          </View>

          <View style={styles.fieldBlock}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Pattern</Text>
            <View style={styles.dayButtonRow}>
              {WEEKDAY_LABELS.map((label, index) => {
                const active = target.patternDays.includes(index);
                return (
                  <View
                    key={`${label}-${index}`}
                    style={[
                      styles.dayButton,
                      {
                        backgroundColor: active ? t.card : "transparent",
                        borderColor: active ? t.userMessage : t.border,
                      },
                    ]}
                  >
                    <Text
                      style={[
                        styles.dayButtonLabel,
                        { color: active ? t.foreground : t.mutedForeground },
                      ]}
                    >
                      {label}
                    </Text>
                  </View>
                );
              })}
            </View>
            {target.spanLabel ? (
              <Text style={[styles.patternHint, { color: t.mutedForeground }]}>
                {target.spanLabel}
              </Text>
            ) : null}
          </View>

          <View style={styles.fieldBlock}>
            <Text style={[styles.fieldLabel, { color: t.mutedForeground }]}>Project</Text>
            <Pressable onPress={onOpenProject} style={styles.linkRow}>
              <Text style={[styles.linkText, { color: t.userMessage }]}>
                {target.item.run.project_dir
                  ? `Open ${formatProjectLabel(target.item.run.project_dir)}`
                  : "No project"}
              </Text>
            </Pressable>
          </View>

          {error ? (
            <Text style={[styles.errorText, { color: t.error }]}>{error}</Text>
          ) : null}

          <View style={styles.sheetActions}>
            <Pressable
              onPress={onSave}
              style={[styles.primaryAction, { backgroundColor: t.userMessage }]}
              disabled={isSaving}
            >
              <Text style={styles.primaryActionLabel}>
                {isSaving ? "Saving..." : "Save"}
              </Text>
            </Pressable>
            <Pressable
              onPress={onWakeNow}
              style={[styles.secondaryAction, { borderColor: t.border }]}
            >
              <Text style={[styles.secondaryActionLabel, { color: t.foreground }]}>
                Wake now
              </Text>
            </Pressable>
            <Pressable
              onPress={onOpenRun}
              style={[styles.secondaryAction, { borderColor: t.border }]}
            >
              <Text style={[styles.secondaryActionLabel, { color: t.foreground }]}>
                Open run
              </Text>
            </Pressable>
          </View>
        </View>
      </View>
    </Modal>
  );
}

export function MakoScheduleView({
  state,
  onSelectRun,
  onOpenProject,
}: MakoScheduleViewProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;

  const scheduledItems = useMemo(
    () => buildScheduledItems(state.current?.runs ?? []),
    [state.current?.runs],
  );
  const seriesDayMap = useMemo(
    () => buildSeriesDayMap(scheduledItems),
    [scheduledItems],
  );

  const firstScheduledDate = scheduledItems[0]?.scheduledAt ?? new Date();
  const [mode, setMode] = useState<ScheduleMode>("month_day");
  const [visibleMonth, setVisibleMonth] = useState(monthStart(firstScheduledDate));
  const [selectedDate, setSelectedDate] = useState(firstScheduledDate);
  const [hasInitializedSelection, setHasInitializedSelection] = useState(false);
  const [detailTarget, setDetailTarget] = useState<ScheduleDetailTarget | null>(null);
  const [detailTitle, setDetailTitle] = useState("");
  const [detailWakeInput, setDetailWakeInput] = useState("");
  const [detailError, setDetailError] = useState<string | null>(null);
  const [isSavingDetail, setIsSavingDetail] = useState(false);
  const [commitments, setCommitments] = useState<MakoGlobalSchedule[]>([]);
  const [commitmentsError, setCommitmentsError] = useState<string | null>(null);
  const [isLoadingCommitments, setIsLoadingCommitments] = useState(true);
  const [mutatingScheduleId, setMutatingScheduleId] = useState<string | null>(null);

  const refreshCommitments = useCallback(async () => {
    if (!client) {
      setCommitments([]);
      setIsLoadingCommitments(false);
      return;
    }
    setCommitmentsError(null);
    try {
      setCommitments(await client.listMakoSchedules({ limit: 200 }));
    } catch (error) {
      setCommitmentsError(
        error instanceof Error ? error.message : "Failed to load Hive schedules.",
      );
    } finally {
      setIsLoadingCommitments(false);
    }
  }, [client]);

  useEffect(() => {
    void refreshCommitments();
  }, [refreshCommitments]);

  const handleScheduleStatusToggle = useCallback(
    async (schedule: MakoGlobalSchedule) => {
      if (!client || !["enabled", "paused"].includes(schedule.status)) {
        return;
      }
      setMutatingScheduleId(schedule.id);
      setCommitmentsError(null);
      try {
        if (schedule.status === "enabled") {
          await client.pauseMakoSchedule(
            schedule.controller_session_id,
            schedule.id,
            schedule.revision,
          );
        } else {
          await client.resumeMakoSchedule(
            schedule.controller_session_id,
            schedule.id,
            schedule.revision,
          );
        }
        await refreshCommitments();
      } catch (error) {
        setCommitmentsError(
          error instanceof Error ? error.message : "Failed to update this schedule.",
        );
      } finally {
        setMutatingScheduleId(null);
      }
    },
    [client, refreshCommitments],
  );

  useEffect(() => {
    if (!detailTarget) {
      setDetailTitle("");
      setDetailWakeInput("");
      setDetailError(null);
      return;
    }
    setDetailTitle(detailTarget.item.title);
    setDetailWakeInput(formatScheduleInputValue(detailTarget.item.wakeAt));
    setDetailError(null);
  }, [detailTarget]);

  useEffect(() => {
    if (hasInitializedSelection || !scheduledItems[0]) {
      return;
    }

    setSelectedDate(scheduledItems[0].scheduledAt);
    setVisibleMonth(monthStart(scheduledItems[0].scheduledAt));
    setHasInitializedSelection(true);
  }, [hasInitializedSelection, scheduledItems]);

  const calendarDays = useMemo(() => buildCalendarDays(visibleMonth), [visibleMonth]);
  const selectedKey = dayKeyFromDate(selectedDate);
  const selectedItems = useMemo(
    () => scheduledItems.filter((item) => item.dayKey === selectedKey),
    [scheduledItems, selectedKey],
  );
  const weekStartDate = useMemo(() => startOfWeek(selectedDate), [selectedDate]);
  const weekDays = useMemo(
    () => Array.from({ length: 7 }, (_, index) => addDays(weekStartDate, index)),
    [weekStartDate],
  );
  const weekLaneRows = useMemo(
    () => packWeekLanes(buildWeekLanes(scheduledItems, weekStartDate)),
    [scheduledItems, weekStartDate],
  );

  const selectedDayCount = selectedItems.length;
  const nextWake =
    commitments.find(
      (schedule) => schedule.status === "enabled" && schedule.next_fire_at,
    )?.next_fire_at ?? null;

  const openItemDetail = (item: ScheduledRunItem, dayCount = 1) => {
    const patternDays = seriesDayMap.get(item.seriesKey) ?? [item.scheduledAt.getDay()];
    setDetailTarget({
      item,
      patternDays,
      spanLabel:
        dayCount > 1
          ? `This series spans ${dayCount} days in the current week.`
          : formatPatternLabel(patternDays),
    });
  };

  const handleSaveDetail = async () => {
    if (!client || !detailTarget) {
      return;
    }

    const nextTitle = detailTitle.trim();
    if (!nextTitle) {
      setDetailError("Title cannot be empty.");
      return;
    }

    const schedule = resolveScheduleSelection("custom", detailWakeInput);
    if (schedule.error || !schedule.startAt) {
      setDetailError(schedule.error ?? "Enter a valid future time.");
      return;
    }

    setIsSavingDetail(true);
    setDetailError(null);
    try {
      if (nextTitle !== detailTarget.item.run.title) {
        await client.updateSession(detailTarget.item.run.session_id, { title: nextTitle });
      }
      if (schedule.startAt !== detailTarget.item.wakeAt) {
        await client.scheduleMakoSession(detailTarget.item.run.session_id, schedule.startAt);
      }
      await state.refresh();
      setDetailTarget(null);
    } catch (error) {
      setDetailError(
        error instanceof Error ? error.message : "Failed to update this schedule item.",
      );
    } finally {
      setIsSavingDetail(false);
    }
  };

  const handleWakeNow = async () => {
    if (!client || !detailTarget) {
      return;
    }
    setIsSavingDetail(true);
    setDetailError(null);
    try {
      await client.resumeMakoSession(detailTarget.item.run.session_id);
      await state.refresh();
      setDetailTarget(null);
    } catch (error) {
      setDetailError(
        error instanceof Error ? error.message : "Failed to wake this run.",
      );
    } finally {
      setIsSavingDetail(false);
    }
  };

  return (
    <>
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator={false}
        refreshControl={
          <RefreshControl
            refreshing={state.isRefreshing || isLoadingCommitments}
            onRefresh={() => {
              void Promise.all([state.refresh(), refreshCommitments()]);
            }}
            tintColor={t.userMessage}
          />
        }
      >
        <Text style={[styles.description, { color: t.mutedForeground }]}>
          What Hive is committed to, when it will wake, and whether each
          commitment is active.
        </Text>

        <View
          style={[
            styles.sectionBlock,
            { borderColor: t.border, backgroundColor: t.card },
          ]}
        >
          <View style={styles.agendaHeader}>
            <View>
              <Text style={[styles.agendaTitle, { color: t.foreground }]}>
                What&apos;s set
              </Text>
              <Text style={[styles.agendaSubtitle, { color: t.mutedForeground }]}>
                Durable one-time and recurring commitments
              </Text>
            </View>
          </View>

          {commitmentsError ? (
            <Text style={[styles.errorText, { color: t.error }]}>
              {commitmentsError}
            </Text>
          ) : null}

          {isLoadingCommitments && commitments.length === 0 ? (
            <ListRowsSkeleton rows={4} />
          ) : commitments.length === 0 ? (
            <Text style={[styles.empty, { color: t.mutedForeground }]}>
              Nothing is scheduled yet.
            </Text>
          ) : (
            <View style={styles.agendaList}>
              {commitments.map((schedule) => (
                <View
                  key={schedule.id}
                  style={[styles.row, { borderColor: t.border }]}
                >
                  <View style={styles.rowCopy}>
                    <Text
                      style={[styles.rowTitle, { color: t.foreground }]}
                      numberOfLines={1}
                    >
                      {schedule.title}
                    </Text>
                    <Text
                      style={[styles.rowDetail, { color: t.mutedForeground }]}
                      numberOfLines={2}
                    >
                      {schedule.summary || schedule.objective}
                    </Text>
                    <View style={styles.metaRow}>
                      <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
                        {recurrenceLabel(schedule)}
                      </Text>
                      <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
                        {nextFireLabel(schedule)}
                      </Text>
                      <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
                        {schedule.status}
                      </Text>
                    </View>
                  </View>
                  {schedule.status === "enabled" || schedule.status === "paused" ? (
                    <Pressable
                      disabled={mutatingScheduleId === schedule.id}
                      onPress={() => {
                        void handleScheduleStatusToggle(schedule);
                      }}
                    >
                      <Text style={[styles.rowAction, { color: t.userMessage }]}>
                        {mutatingScheduleId === schedule.id
                          ? "Saving..."
                          : schedule.status === "enabled"
                            ? "Pause"
                            : "Resume"}
                      </Text>
                    </Pressable>
                  ) : null}
                </View>
              ))}
            </View>
          )}
        </View>

        <Text style={[styles.agendaSubtitle, { color: t.mutedForeground }]}>
          Run wake timeline
        </Text>
        <ScheduleModeToggle mode={mode} onChange={setMode} />

        <View
          style={[
            styles.summaryStrip,
            {
              borderColor: t.border,
            },
          ]}
        >
          <View style={styles.summaryCell}>
            <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>
              Upcoming
            </Text>
            <Text style={[styles.summaryValue, { color: t.foreground }]}>
              {commitments.length}
            </Text>
          </View>
          <View style={[styles.summaryCell, styles.summaryDivider, { borderColor: t.border }]}>
            <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>
              Day plan
            </Text>
            <Text style={[styles.summaryValue, { color: t.foreground }]}>
              {selectedDayCount}
            </Text>
          </View>
          <View style={[styles.summaryCell, styles.summaryDivider, { borderColor: t.border }]}>
            <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>
              Next wake
            </Text>
            <Text style={[styles.summaryValue, { color: t.foreground }]}>
              {nextWake ? timeLabel(nextWake) : "None"}
            </Text>
          </View>
        </View>

        <View style={[styles.sectionBlock, { borderColor: t.border, backgroundColor: t.card }]}>
          <View style={styles.monthHeader}>
            <Pressable
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                setVisibleMonth((current) => shiftMonth(current, -1));
              }}
              style={styles.monthAction}
            >
              <Text style={[styles.monthActionText, { color: t.userMessage }]}>‹</Text>
            </Pressable>
            <Text style={[styles.monthLabel, { color: t.foreground }]}>
              {visibleMonth.toLocaleDateString([], {
                month: "long",
                year: "numeric",
              })}
            </Text>
            <Pressable
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                setVisibleMonth((current) => shiftMonth(current, 1));
              }}
              style={styles.monthAction}
            >
              <Text style={[styles.monthActionText, { color: t.userMessage }]}>›</Text>
            </Pressable>
          </View>

          {mode !== "week" ? (
            <>
              <View style={styles.weekdays}>
                {WEEKDAY_LABELS.map((label, index) => (
                  <Text
                    key={`${label}-${index}`}
                    style={[styles.weekday, { color: t.mutedForeground }]}
                  >
                    {label}
                  </Text>
                ))}
              </View>

              <View style={styles.calendarGrid}>
                {calendarDays.map((date) => {
                  const currentMonth = date.getMonth() === visibleMonth.getMonth();
                  const markerCount = scheduledItems.filter(
                    (item) => item.dayKey === dayKeyFromDate(date),
                  ).length;
                  const active = sameDay(date, selectedDate);
                  return (
                    <Pressable
                      key={date.toISOString()}
                      onPress={() => {
                        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                        setSelectedDate(date);
                      }}
                      style={[
                        styles.dayCell,
                        {
                          borderColor: active ? t.userMessage : t.border,
                          backgroundColor: active ? `${t.userMessage}16` : "transparent",
                        },
                      ]}
                    >
                      <Text
                        style={[
                          styles.dayLabel,
                          { color: currentMonth ? t.foreground : `${t.mutedForeground}99` },
                        ]}
                      >
                        {date.getDate()}
                      </Text>
                      {markerCount > 0 ? (
                        <View
                          style={[
                            styles.dayMarker,
                            { backgroundColor: monthMarkerColor(markerCount, theme.colors) },
                          ]}
                        />
                      ) : null}
                    </Pressable>
                  );
                })}
              </View>
            </>
          ) : (
            <View style={styles.weekBlock}>
              <View style={styles.weekHeaderRow}>
                {weekDays.map((date) => {
                  const active = sameDay(date, selectedDate);
                  return (
                    <Pressable
                      key={date.toISOString()}
                      onPress={() => {
                        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                        setSelectedDate(date);
                      }}
                      style={[
                        styles.weekHeaderCell,
                        active && { backgroundColor: theme.colors.background },
                      ]}
                    >
                      <Text style={[styles.weekHeaderDay, { color: t.mutedForeground }]}>
                        {date.toLocaleDateString([], { weekday: "short" })}
                      </Text>
                      <Text style={[styles.weekHeaderDate, { color: t.foreground }]}>
                        {date.getDate()}
                      </Text>
                    </Pressable>
                  );
                })}
              </View>

              <View style={styles.weekLanes}>
                {weekLaneRows.length === 0 ? (
                  <Text style={[styles.empty, { color: t.mutedForeground }]}>
                    Nothing is scheduled for this week.
                  </Text>
                ) : (
                  weekLaneRows.map((lane, index) => (
                    <View key={`lane-${index}`} style={styles.weekLaneRow}>
                      <View style={styles.weekLaneCells}>
                        {weekDays.map((date) => (
                          <View
                            key={`${date.toISOString()}-${index}`}
                            style={[styles.weekLaneCell, { borderColor: t.border }]}
                          />
                        ))}
                      </View>
                      {lane.map((bar) => {
                        const left: DimensionValue = `${(bar.startIndex / 7) * 100}%`;
                        const width: DimensionValue = `${(bar.span / 7) * 100}%`;
                        return (
                          <Pressable
                            key={bar.id}
                            onPress={() => {
                              openItemDetail(bar.item, bar.dayCount);
                            }}
                            style={[
                              styles.weekBar,
                              {
                                left,
                                width,
                                backgroundColor:
                                  bar.span > 1 ? `${t.userMessage}22` : theme.colors.background,
                                borderColor: bar.span > 1 ? t.userMessage : t.border,
                              },
                            ]}
                          >
                            <Text
                              style={[styles.weekBarText, { color: t.foreground }]}
                              numberOfLines={1}
                            >
                              {`${timeLabel(bar.item.wakeAt)} • ${bar.item.title}`}
                            </Text>
                          </Pressable>
                        );
                      })}
                    </View>
                  ))
                )}
              </View>
            </View>
          )}
        </View>

        {mode === "month" ? (
          <View style={[styles.monthFooter, { borderColor: t.border }]}>
            <View style={styles.monthFooterCopy}>
              <Text style={[styles.monthFooterTitle, { color: t.foreground }]}>
                {dayLabel(selectedDate)}
              </Text>
              <Text style={[styles.monthFooterDetail, { color: t.mutedForeground }]}>
                {selectedDayCount === 0
                  ? "No scheduled work"
                  : `${selectedDayCount} item${selectedDayCount === 1 ? "" : "s"} planned`}
              </Text>
            </View>
            <Pressable
              onPress={() => {
                setMode("month_day");
              }}
            >
              <Text style={[styles.monthFooterLink, { color: t.userMessage }]}>
                View day
              </Text>
            </Pressable>
          </View>
        ) : (
          <View style={[styles.sectionBlock, { borderColor: t.border, backgroundColor: t.card }]}>
            <View style={styles.agendaHeader}>
              <View>
                <Text style={[styles.agendaTitle, { color: t.foreground }]}>
                  {dayLabel(selectedDate)}
                </Text>
                <Text style={[styles.agendaSubtitle, { color: t.mutedForeground }]}>
                  Daily agenda
                </Text>
              </View>
              <Pressable
                onPress={() => {
                  setMode("month");
                }}
              >
                <Text style={[styles.monthFooterLink, { color: t.userMessage }]}>
                  Full month
                </Text>
              </Pressable>
            </View>

            {selectedItems.length === 0 ? (
              <Text style={[styles.empty, { color: t.mutedForeground }]}>
                No work is scheduled for this day.
              </Text>
            ) : (
              <View style={styles.agendaList}>
                {selectedItems.map((item) => (
                  <ScheduleRow
                    key={item.run.session_id}
                    item={item}
                    patternLabel={formatPatternLabel(seriesDayMap.get(item.seriesKey) ?? [])}
                    onPress={() => {
                      openItemDetail(item);
                    }}
                  />
                ))}
              </View>
            )}
          </View>
        )}
      </ScrollView>

      <ScheduleDetailSheet
        visible={detailTarget !== null}
        target={detailTarget}
        titleValue={detailTitle}
        wakeInput={detailWakeInput}
        onChangeTitle={setDetailTitle}
        onChangeWakeInput={setDetailWakeInput}
        onSave={() => {
          void handleSaveDetail();
        }}
        onClose={() => {
          setDetailTarget(null);
        }}
        onWakeNow={() => {
          void handleWakeNow();
        }}
        onOpenRun={() => {
          if (!detailTarget) {
            return;
          }
          setDetailTarget(null);
          onSelectRun(detailTarget.item.run.session_id);
        }}
        onOpenProject={() => {
          const projectDir = detailTarget?.item.run.project_dir;
          const targetBranch = detailTarget?.item.run.target_branch ?? null;
          if (!projectDir || !onOpenProject) {
            return;
          }
          setDetailTarget(null);
          void onOpenProject(projectDir, targetBranch);
        }}
        isSaving={isSavingDetail}
        error={detailError}
      />
    </>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 14,
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  toggleRow: {
    flexDirection: "row",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    padding: 3,
    gap: 4,
  },
  toggleButton: {
    flex: 1,
    minHeight: 36,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
  },
  toggleLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  summaryStrip: {
    flexDirection: "row",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    overflow: "hidden",
  },
  summaryCell: {
    flex: 1,
    paddingHorizontal: 10,
    paddingVertical: 10,
  },
  summaryDivider: {
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  summaryLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  summaryValue: {
    marginTop: 4,
    fontSize: 15,
    fontWeight: "600",
  },
  sectionBlock: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    padding: 10,
    gap: 10,
  },
  monthHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  monthAction: {
    minHeight: 34,
    minWidth: 34,
    alignItems: "center",
    justifyContent: "center",
  },
  monthActionText: {
    fontSize: 18,
    fontWeight: "600",
  },
  monthLabel: {
    fontSize: 15,
    fontWeight: "600",
  },
  weekdays: {
    flexDirection: "row",
  },
  weekday: {
    flex: 1,
    textAlign: "center",
    fontSize: 11,
    fontWeight: "600",
  },
  calendarGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: -2,
  },
  dayCell: {
    width: "14.2857%",
    aspectRatio: 1,
    paddingVertical: 6,
    alignItems: "center",
    justifyContent: "space-between",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
  },
  dayLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  dayMarker: {
    width: 10,
    height: 10,
    borderRadius: 3,
  },
  monthFooter: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  monthFooterCopy: {
    flex: 1,
  },
  monthFooterTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  monthFooterDetail: {
    marginTop: 2,
    fontSize: 12,
  },
  monthFooterLink: {
    fontSize: 12,
    fontWeight: "600",
  },
  weekBlock: {
    gap: 10,
  },
  weekHeaderRow: {
    flexDirection: "row",
    gap: 4,
  },
  weekHeaderCell: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: 3,
    paddingVertical: 7,
    borderRadius: 10,
  },
  weekHeaderDay: {
    fontSize: 11,
    fontWeight: "600",
  },
  weekHeaderDate: {
    fontSize: 13,
    fontWeight: "600",
  },
  weekLanes: {
    gap: 8,
  },
  weekLaneRow: {
    position: "relative",
    minHeight: 38,
    justifyContent: "center",
  },
  weekLaneCells: {
    flexDirection: "row",
    gap: 4,
  },
  weekLaneCell: {
    flex: 1,
    height: 34,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
  },
  weekBar: {
    position: "absolute",
    top: 1,
    bottom: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 8,
    alignItems: "flex-start",
    justifyContent: "center",
  },
  weekBarText: {
    fontSize: 11,
    fontWeight: "600",
  },
  agendaHeader: {
    flexDirection: "row",
    alignItems: "flex-end",
    justifyContent: "space-between",
    gap: 12,
  },
  agendaTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  agendaSubtitle: {
    marginTop: 2,
    fontSize: 12,
  },
  agendaList: {
    gap: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  rowCopy: {
    flex: 1,
    minWidth: 0,
  },
  rowTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  rowDetail: {
    marginTop: 3,
    fontSize: 12,
  },
  metaRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    marginTop: 4,
  },
  rowMeta: {
    fontSize: 12,
  },
  rowAction: {
    fontSize: 12,
    fontWeight: "600",
  },
  empty: {
    paddingVertical: 12,
    fontSize: 14,
    lineHeight: 18,
  },
  sheetOverlay: {
    flex: 1,
    backgroundColor: "rgba(0, 0, 0, 0.42)",
    justifyContent: "flex-end",
    padding: 12,
  },
  sheet: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 18,
    padding: 14,
    gap: 12,
  },
  sheetHeader: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 12,
  },
  sheetCopy: {
    flex: 1,
  },
  sheetTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  sheetSubtitle: {
    marginTop: 3,
    fontSize: 12,
  },
  sheetClose: {
    minHeight: 32,
    justifyContent: "center",
  },
  sheetCloseLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  fieldBlock: {
    gap: 6,
  },
  fieldLabel: {
    fontSize: 11,
    fontWeight: "600",
  },
  fieldInput: {
    minHeight: 42,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 14,
  },
  readonlyValue: {
    fontSize: 14,
    lineHeight: 19,
  },
  dayButtonRow: {
    flexDirection: "row",
    gap: 6,
  },
  dayButton: {
    flex: 1,
    minHeight: 34,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
  },
  dayButtonLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  patternHint: {
    fontSize: 12,
  },
  linkRow: {
    minHeight: 34,
    justifyContent: "center",
  },
  linkText: {
    fontSize: 13,
    fontWeight: "600",
  },
  errorText: {
    fontSize: 12,
    lineHeight: 17,
  },
  sheetActions: {
    flexDirection: "row",
    gap: 8,
  },
  primaryAction: {
    flex: 1,
    minHeight: 40,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  primaryActionLabel: {
    color: "#ffffff",
    fontSize: 13,
    fontWeight: "600",
  },
  secondaryAction: {
    flex: 1,
    minHeight: 40,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  secondaryActionLabel: {
    fontSize: 13,
    fontWeight: "600",
  },
});
