import { useMemo, useState } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type {
  HiveCrewDocumentKind,
  HiveHomeDocumentKind,
} from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import { DetailPaneSkeleton, ListRowsSkeleton } from "../ui/Skeleton";
import { HiveEditorModal } from "./HiveEditorModal";
import { HiveStatusBadge } from "./HiveStatusBadge";
import type { HiveHomeState } from "./types";

interface HivePresenceDetailsProps {
  state: HiveHomeState;
  showChannelsRow?: boolean;
  /**
   * Legacy crew roster rows. The Workers view renders the durable Worker
   * roster itself and hides this transitional section.
   */
  showCrewRoster?: boolean;
}

type EditorTarget =
  | {
      scope: "home";
      kind: HiveHomeDocumentKind;
      title: string;
      subtitle: string;
      initialValue: string;
    }
  | {
      scope: "crew";
      slug: string;
      kind: HiveCrewDocumentKind;
      title: string;
      subtitle: string;
      initialValue: string;
    };

function previewText(value?: string | null, fallback = "Not configured yet.") {
  const trimmed = value?.trim();
  if (!trimmed) {
    return fallback;
  }
  return trimmed;
}

function DetailRow({
  label,
  value,
  actionLabel,
  onAction,
}: {
  label: string;
  value: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.detailRow, { borderColor: t.border }]}>
      <View style={styles.detailCopy}>
        <Text style={[styles.detailLabel, { color: t.mutedForeground }]}>{label}</Text>
        <Text style={[styles.detailValue, { color: t.foreground }]} numberOfLines={3}>
          {value}
        </Text>
      </View>
      {actionLabel && onAction ? (
        <Pressable onPress={onAction} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>
            {actionLabel}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

function CrewMemberRow({
  slug,
  status,
  identity,
  soul,
  memory,
  activitySummary,
  onEditIdentity,
  onEditSoul,
  onEditMemory,
}: {
  slug: string;
  status: string;
  identity?: string | null;
  soul?: string | null;
  memory?: string | null;
  activitySummary?: string | null;
  onEditIdentity: () => void;
  onEditSoul: () => void;
  onEditMemory: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.detailRow, { borderColor: t.border }]}>
      <View style={styles.detailCopy}>
        <View style={styles.crewHeader}>
          <Text style={[styles.crewName, { color: t.foreground }]}>{slug}</Text>
          <HiveStatusBadge status={status} />
        </View>
        <Text style={[styles.detailValue, { color: t.mutedForeground }]} numberOfLines={2}>
          {previewText(identity, "No identity yet.")}
        </Text>
        {activitySummary ? (
          <Text style={[styles.crewMeta, { color: t.mutedForeground }]} numberOfLines={2}>
            {activitySummary}
          </Text>
        ) : null}
        <Text style={[styles.crewMeta, { color: t.mutedForeground }]} numberOfLines={2}>
          Soul: {previewText(soul, "Not configured")} • Memory: {previewText(memory, "Not configured")}
        </Text>
      </View>
      <View style={styles.crewActions}>
        <Pressable onPress={onEditIdentity} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>Identity</Text>
        </Pressable>
        <Pressable onPress={onEditSoul} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>Soul</Text>
        </Pressable>
        <Pressable onPress={onEditMemory} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>Memory</Text>
        </Pressable>
      </View>
    </View>
  );
}

function crewActivitySummary(member: NonNullable<HiveHomeState["crew"]>["members"][number]) {
  const parts: string[] = [];
  if (member.active_run_count > 0) {
    parts.push(`${member.active_run_count} active`);
  }
  if (member.queued_task_count > 0) {
    parts.push(`${member.queued_task_count} queued`);
  }
  if (member.failed_run_count > 0 || member.failed_task_count > 0) {
    parts.push(`${member.failed_run_count + member.failed_task_count} failed`);
  }
  return parts.length > 0 ? parts.join(" • ") : "No active work";
}

export function HivePresenceDetails({
  state,
  showChannelsRow = true,
  showCrewRoster = true,
}: HivePresenceDetailsProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [editorTarget, setEditorTarget] = useState<EditorTarget | null>(null);

  const home = state.home;
  const crew = state.crew?.members ?? [];
  const needsBootstrap = useMemo(
    () =>
      !state.isLoading &&
      !home?.soul &&
      !home?.identity &&
      !home?.heartbeat &&
      !home?.memory &&
      !home?.channels &&
      (home?.crew_count ?? 0) === 0,
    [home, state.isLoading],
  );

  const handleSave = async (content: string) => {
    if (!editorTarget) {
      return;
    }

    if (editorTarget.scope === "home") {
      await state.updateHomeDocument(editorTarget.kind, content);
    } else {
      await state.updateCrewDocument(editorTarget.slug, editorTarget.kind, content);
    }

    setEditorTarget(null);
  };

  return (
    <View style={styles.section}>
      <View style={styles.sectionHeader}>
        <Text style={[styles.sectionTitle, { color: t.foreground }]}>Presence</Text>
        {needsBootstrap ? (
          <Pressable
            onPress={() => {
              void state.bootstrap();
            }}
            style={styles.rowAction}
            disabled={state.isBootstrapping}
          >
            <Text style={[styles.rowActionText, { color: t.userMessage }]}>
              {state.isBootstrapping ? "Initializing..." : "Initialize"}
            </Text>
          </Pressable>
        ) : null}
      </View>

      {state.error ? (
        <Text style={[styles.errorText, { color: t.error }]}>{state.error}</Text>
      ) : null}

      {state.isLoading && !home ? (
        <View style={styles.loading}>
          <DetailPaneSkeleton />
          <ListRowsSkeleton rows={3} />
        </View>
      ) : null}

      {!state.isLoading && needsBootstrap ? (
        <View style={[styles.detailRow, { borderColor: t.border }]}>
          <View style={styles.detailCopy}>
            <Text style={[styles.detailValue, { color: t.foreground }]}>
              Hive home is not initialized yet.
            </Text>
            <Text style={[styles.crewMeta, { color: t.mutedForeground }]}>
              Create the global soul, identity, heartbeat, memory, channels, and default crew files.
            </Text>
          </View>
        </View>
      ) : null}

      {!needsBootstrap ? (
        <>
          <DetailRow
            label="Identity"
            value={previewText(home?.identity?.preview)}
            actionLabel="Edit"
            onAction={() => {
              setEditorTarget({
                scope: "home",
                kind: "identity",
                title: "Edit identity",
                subtitle: "Visible name, presence, and top-level operator identity for Hive.",
                initialValue: home?.identity?.content ?? "",
              });
            }}
          />
          <DetailRow
            label="Soul"
            value={previewText(home?.soul?.preview)}
            actionLabel="Edit"
            onAction={() => {
              setEditorTarget({
                scope: "home",
                kind: "soul",
                title: "Edit soul",
                subtitle: "The voice, stance, and behavioral center of Hive.",
                initialValue: home?.soul?.content ?? "",
              });
            }}
          />
          <DetailRow
            label="Heartbeat"
            value={previewText(home?.heartbeat?.preview)}
            actionLabel="Edit"
            onAction={() => {
              setEditorTarget({
                scope: "home",
                kind: "heartbeat",
                title: "Edit heartbeat",
                subtitle: "Recurring checks and quiet operator habits that keep Hive alive.",
                initialValue: home?.heartbeat?.content ?? "",
              });
            }}
          />
          {showChannelsRow ? (
            <DetailRow
              label="Channels"
              value={previewText(home?.channels?.preview)}
              actionLabel="Edit"
              onAction={() => {
                setEditorTarget({
                  scope: "home",
                  kind: "channels",
                  title: "Edit channels",
                  subtitle: "How Hive routes updates, approvals, and presence across surfaces.",
                  initialValue: home?.channels?.content ?? "",
                });
              }}
            />
          ) : null}
        </>
      ) : null}

      {showCrewRoster ? (
        <>
          <View style={styles.sectionHeader}>
            <Text style={[styles.sectionTitle, { color: t.foreground }]}>Hive Agents</Text>
            <Text style={[styles.countText, { color: t.mutedForeground }]}>
              {crew.length}
            </Text>
          </View>

          {crew.length === 0 ? (
            <View style={[styles.detailRow, { borderColor: t.border }]}>
              <View style={styles.detailCopy}>
                <Text style={[styles.detailValue, { color: t.foreground }]}>
                  No crew members are configured yet.
                </Text>
              </View>
            </View>
          ) : (
            crew.map((member) => (
              <CrewMemberRow
                key={member.slug}
                slug={member.slug}
                status={member.status}
                identity={member.identity?.preview}
                soul={member.soul?.preview}
                memory={member.memory?.preview}
                activitySummary={crewActivitySummary(member)}
                onEditIdentity={() => {
                  setEditorTarget({
                    scope: "crew",
                    slug: member.slug,
                    kind: "identity",
                    title: `Edit ${member.slug} identity`,
                    subtitle: "Name, role, and external presence for this Hive Agent.",
                    initialValue: member.identity?.content ?? "",
                  });
                }}
                onEditSoul={() => {
                  setEditorTarget({
                    scope: "crew",
                    slug: member.slug,
                    kind: "soul",
                    title: `Edit ${member.slug} soul`,
                    subtitle: "How this Hive Agent thinks, writes, and behaves.",
                    initialValue: member.soul?.content ?? "",
                  });
                }}
                onEditMemory={() => {
                  setEditorTarget({
                    scope: "crew",
                    slug: member.slug,
                    kind: "memory",
                    title: `Edit ${member.slug} memory`,
                    subtitle: "Durable notes and role-specific memory for this Hive Agent.",
                    initialValue: member.memory?.content ?? "",
                  });
                }}
              />
            ))
          )}
        </>
      ) : null}

      <HiveEditorModal
        visible={editorTarget !== null}
        title={editorTarget?.title ?? ""}
        subtitle={editorTarget?.subtitle}
        initialValue={editorTarget?.initialValue ?? ""}
        isSaving={state.isSaving}
        onClose={() => {
          setEditorTarget(null);
        }}
        onSave={handleSave}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  section: {
    gap: 10,
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  countText: {
    fontSize: 12,
    fontWeight: "600",
  },
  loading: {
    paddingVertical: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  errorText: {
    fontSize: 12,
    lineHeight: 18,
  },
  detailRow: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 4,
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  detailCopy: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  detailLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
  },
  detailValue: {
    fontSize: 13,
    lineHeight: 18,
  },
  crewMeta: {
    fontSize: 12,
    lineHeight: 17,
  },
  crewHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  crewName: {
    fontSize: 14,
    fontWeight: "600",
    textTransform: "capitalize",
  },
  rowAction: {
    paddingVertical: 4,
  },
  rowActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  crewActions: {
    alignItems: "flex-end",
    gap: 6,
  },
});
