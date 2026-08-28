import { useCallback, useEffect, useRef, useState } from "react";
import {
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { HiveWorker, ModelInfo } from "@mitsuro/api";
import { useThemeContext } from "../../hooks/useTheme";
import { ListRowsSkeleton } from "../ui/Skeleton";
import { HiveEditorModal } from "./HiveEditorModal";
import { HiveWorkerEditorModal } from "./HiveWorkerEditorModal";
import { HivePresenceDetails } from "./HivePresenceDetails";
import type { HiveWorkersState } from "./hooks/useHiveWorkers";
import type { HiveHomeState } from "./types";
import { createWorkerDmNavigationFence } from "./workerDmNavigationFence";
import {
  workerAvatarColor,
  workerInitials,
  workerMetaLine,
} from "./workerAppearance";

interface HiveCrewViewProps {
  state: HiveHomeState;
  workers: HiveWorkersState;
  models: ModelInfo[];
  onOpenWorkerDm: (sessionId: string) => void;
}

type WorkerDocTarget = {
  workerId: string;
  kind: "identity" | "soul";
  title: string;
  subtitle: string;
  initialValue: string;
};

function WorkerRow({
  worker,
  onOpen,
  onEdit,
  onEditIdentity,
  onEditSoul,
  onRetryIntroduction,
  onSkipIntroduction,
  isSaving,
}: {
  worker: HiveWorker;
  onOpen: () => void;
  onEdit: () => void;
  onEditIdentity: () => void;
  onEditSoul: () => void;
  onRetryIntroduction: () => void;
  onSkipIntroduction: () => void;
  isSaving: boolean;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const color = workerAvatarColor(worker);
  const paused = worker.status === "paused";
  const working = worker.dm_agent_state === "running";
  const introductionNeedsRecovery = worker.introduction_status === "failed" ||
    worker.introduction_status === "needs_recovery";
  const introductionRunning = worker.introduction_status === "queued" ||
    worker.introduction_status === "running";
  const introductionGathering =
    worker.introduction_status === "awaiting_context";
  const statusLabel = introductionNeedsRecovery
    ? "Needs attention"
    : introductionRunning
    ? "Waking up"
    : introductionGathering
    ? "Setting up"
    : paused
    ? "Paused"
    : working
    ? "Working"
    : "Active";
  const statusColor = introductionNeedsRecovery
    ? t.error
    : introductionRunning || introductionGathering || paused
    ? t.warning
    : working
    ? t.success
    : t.mutedForeground;

  return (
    <View style={[styles.workerRow, { borderColor: t.border }]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Open DM with ${worker.display_name}`}
        onPress={onOpen}
        style={styles.workerMain}
      >
        <View
          style={[
            styles.avatar,
            { backgroundColor: `${color}22`, borderColor: `${color}55` },
          ]}
        >
          <Text style={[styles.avatarText, { color }]}>
            {workerInitials(worker.display_name)}
          </Text>
        </View>
        <View style={styles.workerCopy}>
          <View style={styles.workerHeader}>
            <Text
              style={[styles.workerName, { color: t.foreground }]}
              numberOfLines={1}
            >
              {worker.display_name}
            </Text>
            <View style={styles.statusChip}>
              <View
                style={[styles.statusDot, { backgroundColor: statusColor }]}
              />
              <Text style={[styles.statusText, { color: statusColor }]}>
                {statusLabel}
              </Text>
            </View>
          </View>
          <Text
            style={[styles.workerMeta, { color: t.mutedForeground }]}
            numberOfLines={1}
          >
            {workerMetaLine(worker)}
          </Text>
        </View>
      </Pressable>
      {introductionNeedsRecovery && worker.introduction_last_error
        ? (
          <Text
            style={[styles.introductionError, { color: t.error }]}
            numberOfLines={2}
          >
            {worker.introduction_last_error}
          </Text>
        )
        : null}
      <View style={styles.workerActions}>
        {introductionNeedsRecovery
          ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Retry ${worker.display_name} Introduction`}
              disabled={isSaving}
              onPress={onRetryIntroduction}
              style={styles.rowAction}
            >
              <Text style={[styles.rowActionText, { color: t.userMessage }]}>
                Retry
              </Text>
            </Pressable>
          )
          : null}
        {introductionNeedsRecovery || introductionRunning ||
            introductionGathering
          ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Skip ${worker.display_name} Introduction`}
              disabled={isSaving}
              onPress={onSkipIntroduction}
              style={styles.rowAction}
            >
              <Text
                style={[styles.rowActionText, { color: t.mutedForeground }]}
              >
                Skip
              </Text>
            </Pressable>
          )
          : null}
        <Pressable onPress={onEditIdentity} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>
            Identity
          </Text>
        </Pressable>
        <Pressable onPress={onEditSoul} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>
            Soul
          </Text>
        </Pressable>
        <Pressable onPress={onEdit} style={styles.rowAction}>
          <Text style={[styles.rowActionText, { color: t.userMessage }]}>
            Edit
          </Text>
        </Pressable>
      </View>
    </View>
  );
}

/**
 * Hive Workers roster: durable identities with their own persona, model, and
 * private DM lane. The Hive home persona documents remain editable below.
 */
export function HiveCrewView({
  state,
  workers,
  models,
  onOpenWorkerDm,
}: HiveCrewViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [editorTarget, setEditorTarget] = useState<
    { mode: "create" } | { mode: "edit"; workerId: string } | null
  >(null);
  const [docTarget, setDocTarget] = useState<WorkerDocTarget | null>(null);
  const openingWorkerIdRef = useRef<string | null>(null);
  const workerDmNavigationFenceRef = useRef(
    createWorkerDmNavigationFence(),
  );
  const editingWorker = editorTarget?.mode === "edit"
    ? workers.workers.find((worker) => worker.id === editorTarget.workerId) ??
      null
    : null;

  useEffect(() => {
    const fence = workerDmNavigationFenceRef.current;
    fence.mount();
    return () => fence.unmount();
  }, []);

  const invalidateWorkerDmNavigation = useCallback(() => {
    workerDmNavigationFenceRef.current.invalidate();
  }, []);

  const openWorkerDm = async (worker: HiveWorker) => {
    const intent = workerDmNavigationFenceRef.current.beginIntent();
    if (openingWorkerIdRef.current) {
      return;
    }
    openingWorkerIdRef.current = worker.id;
    try {
      const dm = await workers.ensureWorkerDm(worker.id);
      if (dm && workerDmNavigationFenceRef.current.isCurrent(intent)) {
        onOpenWorkerDm(dm.session_id);
      }
    } finally {
      if (openingWorkerIdRef.current === worker.id) {
        openingWorkerIdRef.current = null;
      }
    }
  };

  const openDocEditor = async (
    worker: HiveWorker,
    kind: "identity" | "soul",
  ) => {
    invalidateWorkerDmNavigation();
    const detail = await workers.loadWorkerDetail(worker.id);
    if (!detail) {
      return;
    }
    setDocTarget({
      workerId: worker.id,
      kind,
      title: `Edit ${worker.display_name} ${kind}`,
      subtitle: kind === "identity"
        ? "Name, role, and outward presence for this Worker."
        : "How this Worker thinks, writes, and behaves.",
      initialValue: (kind === "identity" ? detail.identity : detail.soul) ?? "",
    });
  };

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.wrap}
      showsVerticalScrollIndicator={false}
      refreshControl={
        <RefreshControl
          refreshing={state.isRefreshing || workers.isRefreshing}
          onRefresh={() => {
            void state.refresh();
            void workers.refresh();
          }}
          tintColor={t.userMessage}
        />
      }
    >
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Hive Workers are durable teammates. Each Worker carries its own
        identity, soul, model, and a private DM — open a Worker to talk to it
        directly.
      </Text>

      <View style={styles.section}>
        <View style={styles.sectionHeader}>
          <Text style={[styles.sectionTitle, { color: t.foreground }]}>
            Workers
          </Text>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Create a new Worker"
            onPress={() => {
              invalidateWorkerDmNavigation();
              setEditorTarget({ mode: "create" });
            }}
            style={styles.rowAction}
          >
            <Text style={[styles.rowActionText, { color: t.userMessage }]}>
              New Worker
            </Text>
          </Pressable>
        </View>

        {workers.error
          ? (
            <Text style={[styles.errorText, { color: t.error }]}>
              {workers.error}
            </Text>
          )
          : null}

        {(workers.isLoading || workers.isRefreshing) &&
            workers.workers.length === 0
          ? <ListRowsSkeleton rows={3} />
          : null}

        {!workers.isLoading && !workers.isRefreshing &&
            workers.workers.length === 0
          ? (
            <View style={[styles.workerRow, { borderColor: t.border }]}>
              <Text style={[styles.workerMeta, { color: t.mutedForeground }]}>
                No Workers yet. Create one to give the Hive a dedicated
                teammate.
              </Text>
            </View>
          )
          : null}

        {workers.workers.map((worker) => (
          <WorkerRow
            key={worker.id}
            worker={worker}
            onOpen={() => {
              void openWorkerDm(worker);
            }}
            onEdit={() => {
              invalidateWorkerDmNavigation();
              setEditorTarget({ mode: "edit", workerId: worker.id });
            }}
            onEditIdentity={() => {
              void openDocEditor(worker, "identity");
            }}
            onEditSoul={() => {
              void openDocEditor(worker, "soul");
            }}
            onRetryIntroduction={() => {
              const intent = workerDmNavigationFenceRef.current.beginIntent();
              void workers
                .retryIntroduction(worker.id)
                .then((detail) => {
                  if (
                    detail.dm_session_id &&
                    workerDmNavigationFenceRef.current.isCurrent(intent)
                  ) {
                    onOpenWorkerDm(detail.dm_session_id);
                  }
                })
                // The hook already projects the failure into workers.error.
                // Consume the event-handler promise so a visible API failure
                // does not also become an unhandled rejection.
                .catch(() => undefined);
            }}
            onSkipIntroduction={() => {
              invalidateWorkerDmNavigation();
              void workers.skipIntroduction(worker.id).catch(() => undefined);
            }}
            isSaving={workers.isSaving}
          />
        ))}
      </View>

      <HivePresenceDetails
        state={state}
        showChannelsRow={false}
        showCrewRoster={false}
      />

      <HiveWorkerEditorModal
        visible={editorTarget?.mode === "create" || editingWorker !== null}
        worker={editingWorker}
        models={models}
        isSaving={workers.isSaving}
        onClose={() => {
          invalidateWorkerDmNavigation();
          setEditorTarget(null);
        }}
        onCreate={async (request) => {
          const intent = workerDmNavigationFenceRef.current.beginIntent();
          const created = await workers.createWorker(request);
          setEditorTarget(null);
          if (
            created.dm_session_id &&
            workerDmNavigationFenceRef.current.isCurrent(intent)
          ) {
            onOpenWorkerDm(created.dm_session_id);
          }
        }}
        onUpdate={async (id, request) => {
          await workers.updateWorker(id, request);
          setEditorTarget(null);
        }}
        onPause={async (id) => {
          await workers.pauseWorker(id);
          setEditorTarget(null);
        }}
        onResume={async (id) => {
          await workers.resumeWorker(id);
          setEditorTarget(null);
        }}
        onArchive={async (id) => {
          await workers.archiveWorker(id);
          setEditorTarget(null);
        }}
      />

      <HiveEditorModal
        visible={docTarget !== null}
        title={docTarget?.title ?? ""}
        subtitle={docTarget?.subtitle}
        initialValue={docTarget?.initialValue ?? ""}
        isSaving={workers.isSaving}
        onClose={() => setDocTarget(null)}
        onSave={async (content) => {
          if (!docTarget) {
            return;
          }
          await workers.updateWorker(docTarget.workerId, {
            [docTarget.kind]: content,
          });
          setDocTarget(null);
        }}
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  wrap: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 14,
  },
  description: {
    fontSize: 12,
    lineHeight: 18,
  },
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
  errorText: {
    fontSize: 12,
    lineHeight: 18,
  },
  workerRow: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 4,
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  workerMain: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  avatar: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  avatarText: {
    fontSize: 13,
    fontWeight: "700",
  },
  workerCopy: {
    flex: 1,
    minWidth: 0,
    gap: 3,
  },
  workerHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  workerName: {
    fontSize: 14,
    fontWeight: "600",
    flexShrink: 1,
  },
  statusChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  statusDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  statusText: {
    fontSize: 11,
    fontWeight: "600",
  },
  workerMeta: {
    fontSize: 12,
    lineHeight: 17,
  },
  workerActions: {
    alignItems: "flex-end",
    gap: 6,
  },
  introductionError: {
    fontSize: 11,
    lineHeight: 16,
    paddingLeft: 50,
  },
  rowAction: {
    paddingVertical: 4,
  },
  rowActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
});
