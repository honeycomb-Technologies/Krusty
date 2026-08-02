import { useState } from "react";
import {
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useThemeContext } from "../../hooks/useTheme";
import { ChannelsSkeleton } from "../ui/Skeleton";
import { HiveEditorModal } from "./HiveEditorModal";
import { HiveStatusBadge } from "./HiveStatusBadge";
import { useHiveChannels } from "./hooks/useHiveChannels";
import type { HiveHomeState } from "./types";

interface HiveChannelsViewProps {
  state: HiveHomeState;
}

function ChannelRow({
  label,
  kind,
  detail,
  source,
  status,
}: {
  label: string;
  kind: string;
  detail: string;
  source: string;
  status: string;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.row, { borderColor: t.border }]}>
      <View style={styles.copy}>
        <View style={styles.rowHeader}>
          <Text style={[styles.rowTitle, { color: t.foreground }]}>{label}</Text>
          <HiveStatusBadge status={status} />
        </View>
        <Text style={[styles.rowMeta, { color: t.mutedForeground }]}>
          {kind.replace(/_/g, " ")} • {source}
        </Text>
        <Text style={[styles.rowDetail, { color: t.mutedForeground }]}>{detail}</Text>
      </View>
    </View>
  );
}

export function HiveChannelsView({ state }: HiveChannelsViewProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const channelsState = useHiveChannels(true);
  const [editorOpen, setEditorOpen] = useState(false);

  const home = state.home;
  const combinedError = channelsState.error ?? state.error;
  const needsBootstrap =
    !state.isLoading &&
    !home?.soul &&
    !home?.identity &&
    !home?.heartbeat &&
    !home?.memory &&
    !home?.channels &&
    (home?.crew_count ?? 0) === 0;

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.wrap}
      showsVerticalScrollIndicator={false}
      refreshControl={
        <RefreshControl
          refreshing={channelsState.isRefreshing || state.isRefreshing}
          onRefresh={() => {
            void Promise.all([channelsState.refresh(), state.refresh()]);
          }}
          tintColor={t.userMessage}
        />
      }
    >
      <View style={styles.header}>
        <View style={styles.headerCopy}>
          <Text style={[styles.description, { color: t.mutedForeground }]}>
            Channels defines how Hive speaks outside the immediate run state. The main thread stays primary, while push and other routes surface only when they are actually usable.
          </Text>
        </View>
        <Pressable
          onPress={() => {
            if (needsBootstrap) {
              void state.bootstrap();
            } else {
              setEditorOpen(true);
            }
          }}
          style={styles.headerAction}
        >
          <Text style={[styles.headerActionText, { color: t.userMessage }]}>
            {needsBootstrap
              ? state.isBootstrapping
                ? "Initializing..."
                : "Initialize"
              : "Edit routing"}
          </Text>
        </Pressable>
      </View>

      {combinedError ? (
        <Text style={[styles.errorText, { color: t.error }]}>{combinedError}</Text>
      ) : null}

      <View
        style={[
          styles.summaryStrip,
          { borderTopColor: t.border, borderBottomColor: t.border },
        ]}
      >
        <View style={styles.summaryCell}>
          <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>APNs</Text>
          <Text style={[styles.summaryValue, { color: t.foreground }]}>
            {channelsState.channels?.apns_configured ? "configured" : "offline"}
          </Text>
        </View>
        <View style={styles.summaryCell}>
          <Text style={[styles.summaryLabel, { color: t.mutedForeground }]}>Devices</Text>
          <Text style={[styles.summaryValue, { color: t.foreground }]}>
            {String(channelsState.channels?.apns_device_count ?? 0)}
          </Text>
        </View>
      </View>

      {channelsState.isLoading && !channelsState.channels ? (
        <ChannelsSkeleton />
      ) : null}

      {channelsState.channels?.items.map((item) => (
        <ChannelRow
          key={item.id}
          label={item.label}
          kind={item.kind}
          detail={item.detail}
          source={item.source}
          status={item.status}
        />
      ))}

      {!channelsState.isLoading && !channelsState.channels?.items.length ? (
        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
          No channels are described yet. Initialize or edit routing to define how Hive speaks.
        </Text>
      ) : null}

      <HiveEditorModal
        visible={editorOpen}
        title="Edit channels"
        subtitle="Define how Hive routes updates, approvals, and presence across surfaces."
        initialValue={home?.channels?.content ?? ""}
        isSaving={state.isSaving}
        onClose={() => {
          setEditorOpen(false);
        }}
        onSave={async (content) => {
          await state.updateHomeDocument("channels", content);
          await channelsState.refresh();
          setEditorOpen(false);
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
    gap: 12,
  },
  header: {
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 12,
  },
  headerCopy: {
    flex: 1,
  },
  description: {
    fontSize: 12,
    lineHeight: 18,
  },
  headerAction: {
    paddingVertical: 2,
  },
  headerActionText: {
    fontSize: 12,
    fontWeight: "600",
  },
  errorText: {
    fontSize: 12,
    lineHeight: 18,
  },
  summaryStrip: {
    flexDirection: "row",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingVertical: 10,
    gap: 18,
  },
  summaryCell: {
    gap: 2,
  },
  summaryLabel: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
  },
  summaryValue: {
    fontSize: 14,
    fontWeight: "600",
  },
  loading: {
    paddingVertical: 28,
    alignItems: "center",
    justifyContent: "center",
  },
  row: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 10,
    paddingBottom: 4,
  },
  copy: {
    gap: 3,
  },
  rowHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  rowTitle: {
    flex: 1,
    fontSize: 14,
    fontWeight: "600",
  },
  rowMeta: {
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
    letterSpacing: 0.45,
  },
  rowDetail: {
    fontSize: 12,
    lineHeight: 17,
  },
  emptyText: {
    fontSize: 12,
    lineHeight: 18,
    paddingVertical: 8,
  },
});
