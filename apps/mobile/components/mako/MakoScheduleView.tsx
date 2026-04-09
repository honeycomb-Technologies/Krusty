import { useMemo, useState } from "react";
import {
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoCurrentRunSummary } from "@krusty/api";
import type { MakoCurrentState } from "./types";
import { formatProjectLabel, getRunNextWakeAt } from "./utils";

interface MakoScheduleViewProps {
  state: MakoCurrentState;
  onSelectRun: (runId: string) => void;
}

type ScheduleMode = "agenda" | "calendar";

function dayKey(value: string): string {
  const date = new Date(value);
  return dayKeyFromDate(date);
}

function dayKeyFromDate(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function dayLabel(value: string): string {
  return new Date(value).toLocaleDateString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

function timeLabel(value: string): string {
  return new Date(value).toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
}

function buildAgendaBuckets(runs: MakoCurrentRunSummary[]) {
  const grouped = new Map<string, MakoCurrentRunSummary[]>();
  for (const run of runs) {
    const wakeAt = getRunNextWakeAt(run);
    if (!wakeAt) {
      continue;
    }
    const key = dayKey(wakeAt);
    const bucket = grouped.get(key) ?? [];
    bucket.push(run);
    grouped.set(key, bucket);
  }

  return [...grouped.entries()]
    .sort((left, right) => left[0].localeCompare(right[0]))
    .map(([key, entries]) => ({
      key,
      label: dayLabel(`${key}T00:00:00`),
      entries: entries.sort((left, right) => {
        const leftValue = getRunNextWakeAt(left) ?? "";
        const rightValue = getRunNextWakeAt(right) ?? "";
        return leftValue.localeCompare(rightValue);
      }),
    }));
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

function buildCalendarDays(baseMonth: Date): Date[] {
  const start = monthStart(baseMonth);
  const offset = start.getDay();
  const gridStart = new Date(start);
  gridStart.setDate(start.getDate() - offset);

  return Array.from({ length: 42 }, (_, index) => {
    const date = new Date(gridStart);
    date.setDate(gridStart.getDate() + index);
    return date;
  });
}

function ScheduleRow({
  run,
  onSelect,
}: {
  run: MakoCurrentRunSummary;
  onSelect: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const wakeAt = getRunNextWakeAt(run);

  return (
    <Pressable
      onPress={() => {
        void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        onSelect();
      }}
      style={[styles.row, { borderColor: t.border }]}
    >
      <View style={styles.rowCopy}>
        <Text style={[styles.rowTitle, { color: t.foreground }]} numberOfLines={1}>
          {run.title || "Untitled run"}
        </Text>
        <Text style={[styles.rowMeta, { color: t.mutedForeground }]} numberOfLines={1}>
          {formatProjectLabel(run.project_dir)}
        </Text>
      </View>
      <View style={styles.rowAside}>
        <Text style={[styles.rowTime, { color: t.foreground }]}>
          {wakeAt ? timeLabel(wakeAt) : "Later"}
        </Text>
        <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>Open</Text>
      </View>
    </Pressable>
  );
}

export function MakoScheduleView({
  state,
  onSelectRun,
}: MakoScheduleViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [mode, setMode] = useState<ScheduleMode>("agenda");

  const runs = useMemo(
    () =>
      (state.current?.runs ?? [])
        .filter((run) => Boolean(getRunNextWakeAt(run)))
        .sort((left, right) => {
          const leftValue = getRunNextWakeAt(left) ?? "";
          const rightValue = getRunNextWakeAt(right) ?? "";
          return leftValue.localeCompare(rightValue);
        }),
    [state.current?.runs],
  );

  const agenda = useMemo(() => buildAgendaBuckets(runs), [runs]);
  const firstScheduledDate = runs[0]
    ? new Date(getRunNextWakeAt(runs[0]) as string)
    : new Date();
  const [visibleMonth, setVisibleMonth] = useState(monthStart(firstScheduledDate));
  const [selectedDate, setSelectedDate] = useState<Date>(monthStart(firstScheduledDate));

  const calendarDays = useMemo(() => buildCalendarDays(visibleMonth), [visibleMonth]);
  const selectedKey = dayKeyFromDate(selectedDate);
  const selectedRuns = runs.filter((run) => {
    const wakeAt = getRunNextWakeAt(run);
    return wakeAt ? dayKey(wakeAt) === selectedKey : false;
  });

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
      showsVerticalScrollIndicator={false}
      refreshControl={
        <RefreshControl
          refreshing={state.isRefreshing}
          onRefresh={() => {
            void state.refresh();
          }}
          tintColor={t.userMessage}
        />
      }
    >
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Schedule keeps future work visible across time. Use agenda to scan quickly or calendar to place runs on a real date.
      </Text>

      <View style={[styles.modeSwitch, { borderColor: t.border }]}>
        {([
          { id: "agenda", label: "Agenda" },
          { id: "calendar", label: "Calendar" },
        ] as const).map((item, index) => {
          const active = item.id === mode;
          return (
            <Pressable
              key={item.id}
              onPress={() => {
                if (!active) {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setMode(item.id);
                }
              }}
              style={[
                styles.modeButton,
                {
                  backgroundColor: active ? t.glass.backgroundElevated : "transparent",
                  borderRightWidth: index === 0 ? StyleSheet.hairlineWidth : 0,
                  borderRightColor: t.border,
                },
              ]}
            >
              <Text
                style={[
                  styles.modeLabel,
                  { color: active ? t.foreground : t.mutedForeground },
                ]}
              >
                {item.label}
              </Text>
            </Pressable>
          );
        })}
      </View>

      <View
        style={[
          styles.summaryStrip,
          {
            borderTopColor: t.border,
            borderBottomColor: t.border,
          },
        ]}
      >
        <View style={styles.summaryCell}>
          <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>
            Scheduled
          </Text>
          <Text style={[styles.summaryValue, { color: t.foreground }]}>
            {runs.length}
          </Text>
        </View>
        <View style={styles.summaryCell}>
          <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>
            Next
          </Text>
          <Text style={[styles.summaryValue, { color: t.foreground }]}>
            {runs[0] && getRunNextWakeAt(runs[0])
              ? timeLabel(getRunNextWakeAt(runs[0]) as string)
              : "None"}
          </Text>
        </View>
      </View>

      {mode === "agenda" ? (
        <View style={styles.section}>
          {agenda.length === 0 ? (
            <Text style={[styles.empty, { color: t.mutedForeground }]}>
              No scheduled runs yet.
            </Text>
          ) : (
            agenda.map((bucket) => (
              <View key={bucket.key} style={styles.bucket}>
                <Text style={[styles.bucketTitle, { color: t.foreground }]}>
                  {bucket.label}
                </Text>
                <View style={[styles.bucketBody, { borderTopColor: t.border }]}>
                  {bucket.entries.map((run) => (
                    <ScheduleRow
                      key={run.session_id}
                      run={run}
                      onSelect={() => {
                        onSelectRun(run.session_id);
                      }}
                    />
                  ))}
                </View>
              </View>
            ))
          )}
        </View>
      ) : (
        <View style={styles.section}>
          <View style={styles.monthHeader}>
            <Pressable
              onPress={() => {
                setVisibleMonth((current) => shiftMonth(current, -1));
              }}
              style={styles.monthAction}
            >
              <Text style={[styles.monthActionText, { color: t.userMessage }]}>Prev</Text>
            </Pressable>
            <Text style={[styles.monthLabel, { color: t.foreground }]}>
              {visibleMonth.toLocaleDateString([], {
                month: "long",
                year: "numeric",
              })}
            </Text>
            <Pressable
              onPress={() => {
                setVisibleMonth((current) => shiftMonth(current, 1));
              }}
              style={styles.monthAction}
            >
              <Text style={[styles.monthActionText, { color: t.userMessage }]}>Next</Text>
            </Pressable>
          </View>

          <View style={styles.weekdays}>
            {["S", "M", "T", "W", "T", "F", "S"].map((label, index) => (
              <Text key={`${label}-${index}`} style={[styles.weekday, { color: t.mutedForeground }]}>
                {label}
              </Text>
            ))}
          </View>

          <View style={styles.calendarGrid}>
            {calendarDays.map((date) => {
              const currentMonth = date.getMonth() === visibleMonth.getMonth();
              const hasRun = runs.some((run) => {
                const wakeAt = getRunNextWakeAt(run);
                return wakeAt ? dayKey(wakeAt) === dayKeyFromDate(date) : false;
              });
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
                      { color: currentMonth ? t.foreground : t.mutedForeground },
                    ]}
                  >
                    {date.getDate()}
                  </Text>
                  {hasRun ? (
                    <View
                      style={[
                        styles.dayDot,
                        { backgroundColor: active ? t.userMessage : t.mutedForeground },
                      ]}
                    />
                  ) : null}
                </Pressable>
              );
            })}
          </View>

          <View style={styles.bucket}>
            <Text style={[styles.bucketTitle, { color: t.foreground }]}>
              {selectedDate.toLocaleDateString([], {
                weekday: "long",
                month: "short",
                day: "numeric",
              })}
            </Text>
            <View style={[styles.bucketBody, { borderTopColor: t.border }]}>
              {selectedRuns.length === 0 ? (
                <Text style={[styles.empty, { color: t.mutedForeground }]}>
                  Nothing scheduled for this day.
                </Text>
              ) : (
                selectedRuns.map((run) => (
                  <ScheduleRow
                    key={run.session_id}
                    run={run}
                    onSelect={() => {
                      onSelectRun(run.session_id);
                    }}
                  />
                ))
              )}
            </View>
          </View>
        </View>
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 16,
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  modeSwitch: {
    flexDirection: "row",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    overflow: "hidden",
  },
  modeButton: {
    flex: 1,
    minHeight: 40,
    alignItems: "center",
    justifyContent: "center",
  },
  modeLabel: {
    fontSize: 13,
    fontWeight: "600",
  },
  summaryStrip: {
    flexDirection: "row",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  summaryCell: {
    flex: 1,
    paddingVertical: 10,
    paddingHorizontal: 10,
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
  section: {
    gap: 16,
  },
  bucket: {
    gap: 8,
  },
  bucketTitle: {
    fontSize: 15,
    fontWeight: "600",
  },
  bucketBody: {
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  row: {
    flexDirection: "row",
    gap: 12,
    alignItems: "center",
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  rowCopy: {
    flex: 1,
    minWidth: 0,
  },
  rowTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  rowMeta: {
    marginTop: 3,
    fontSize: 12,
    lineHeight: 16,
  },
  rowAside: {
    alignItems: "flex-end",
    gap: 4,
  },
  rowTime: {
    fontSize: 13,
    fontWeight: "600",
  },
  empty: {
    paddingVertical: 12,
    fontSize: 14,
    lineHeight: 19,
  },
  monthHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  monthAction: {
    minHeight: 32,
    justifyContent: "center",
  },
  monthActionText: {
    fontSize: 12,
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
  },
  dayCell: {
    width: "14.2857%",
    aspectRatio: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 0,
    alignItems: "center",
    justifyContent: "center",
    gap: 4,
  },
  dayLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  dayDot: {
    width: 4,
    height: 4,
    borderRadius: 2,
  },
});
