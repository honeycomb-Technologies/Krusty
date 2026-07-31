import { useCallback, useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { ChevronDown, ChevronRight, FileCode2, RefreshCw } from "lucide-react-native";
import type {
  GitChangedFile,
  GitFileDiffResponse,
  GitStatusResponse,
} from "@krusty/api";

import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { ToolDiffViewer } from "../chat/ToolDiffViewer";

interface ToolboxChangesProps {
  visible: boolean;
  projectDirectory?: string | null;
}

export function ToolboxChanges({
  visible,
  projectDirectory,
}: ToolboxChangesProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [status, setStatus] = useState<GitStatusResponse | null>(null);
  const [files, setFiles] = useState<GitChangedFile[]>([]);
  const [expandedPath, setExpandedPath] = useState<string | null>(null);
  const [diffs, setDiffs] = useState<Record<string, GitFileDiffResponse>>({});
  const [diffLoadingPath, setDiffLoadingPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !visible) {
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const [nextStatus, nextChanges] = await Promise.all([
        client.getGitStatus(projectDirectory ?? undefined),
        client.getGitChanges(projectDirectory ?? undefined),
      ]);
      setStatus(nextStatus);
      setFiles(nextChanges.files);
      setDiffs({});
      setExpandedPath(null);
    } catch (nextError) {
      setError(
        nextError instanceof Error
          ? nextError.message
          : "Unable to load repository changes.",
      );
    } finally {
      setLoading(false);
    }
  }, [client, projectDirectory, visible]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggleFile = useCallback(
    async (file: GitChangedFile) => {
      if (expandedPath === file.path) {
        setExpandedPath(null);
        return;
      }
      setExpandedPath(file.path);
      setDiffError(null);
      if (!client || diffs[file.path]) {
        return;
      }
      setDiffLoadingPath(file.path);
      try {
        const diff = await client.getGitFileDiff(
          file.path,
          projectDirectory ?? undefined,
        );
        setDiffs((current) => ({ ...current, [file.path]: diff }));
      } catch (nextError) {
        setDiffError(
          nextError instanceof Error ? nextError.message : "Unable to load this diff.",
        );
      } finally {
        setDiffLoadingPath(null);
      }
    },
    [client, diffs, expandedPath, projectDirectory],
  );

  const totalAdditions = files.reduce((sum, file) => sum + file.additions, 0);
  const totalDeletions = files.reduce((sum, file) => sum + file.deletions, 0);

  return (
    <ScrollView
      contentContainerStyle={styles.content}
      showsVerticalScrollIndicator={false}
    >
      <View style={styles.headingRow}>
        <View style={styles.headingCopy}>
          <Text style={[styles.title, { color: t.foreground }]}>Changes</Text>
          <Text
            numberOfLines={2}
            style={[styles.subtitle, { color: t.mutedForeground }]}
          >
            {status?.repo_root ?? projectDirectory ?? "No active project"}
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Refresh changes"
          disabled={loading}
          onPress={() => void refresh()}
          style={[styles.refreshButton, { borderColor: t.border }]}
        >
          {loading ? (
            <ActivityIndicator size="small" color={t.mutedForeground} />
          ) : (
            <RefreshCw size={16} color={t.foreground} strokeWidth={1.9} />
          )}
        </Pressable>
      </View>

      {error ? <Text style={[styles.error, { color: t.error }]}>{error}</Text> : null}
      {status && !status.in_repo ? (
        <Text style={[styles.empty, { color: t.mutedForeground }]}>
          The active directory is not a Git repository.
        </Text>
      ) : null}

      {status?.in_repo ? (
        <>
          <View style={[styles.branchRow, { borderColor: t.border }]}>
            <View style={styles.branchCopy}>
              <Text numberOfLines={1} style={[styles.branch, { color: t.foreground }]}>
                {status.branch ?? "Detached HEAD"}
              </Text>
              <Text style={[styles.detail, { color: t.mutedForeground }]}>
                {files.length} file{files.length === 1 ? "" : "s"}
              </Text>
            </View>
            <Text style={styles.diffTotals}>
              <Text style={{ color: t.success }}>+{totalAdditions}</Text>
              <Text style={{ color: t.mutedForeground }}>  </Text>
              <Text style={{ color: t.error }}>−{totalDeletions}</Text>
            </Text>
          </View>

          {files.length === 0 && !loading ? (
            <Text style={[styles.empty, { color: t.mutedForeground }]}>
              No changes from this branch&apos;s base.
            </Text>
          ) : (
            <View style={[styles.fileList, { borderColor: t.border }]}>
              {files.map((file, index) => {
                const expanded = expandedPath === file.path;
                const diff = diffs[file.path];
                return (
                  <View
                    key={file.path}
                    style={[
                      styles.fileItem,
                      index > 0 && { borderTopColor: t.border, borderTopWidth: StyleSheet.hairlineWidth },
                    ]}
                  >
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel={`${expanded ? "Collapse" : "Expand"} ${file.path}`}
                      onPress={() => void toggleFile(file)}
                      style={styles.fileRow}
                    >
                      {expanded ? (
                        <ChevronDown size={16} color={t.mutedForeground} />
                      ) : (
                        <ChevronRight size={16} color={t.mutedForeground} />
                      )}
                      <FileCode2 size={16} color={t.mutedForeground} strokeWidth={1.7} />
                      <View style={styles.fileCopy}>
                        <Text numberOfLines={1} style={[styles.filePath, { color: t.foreground }]}>
                          {file.path}
                        </Text>
                        <Text style={[styles.fileStatus, { color: t.mutedForeground }]}>
                          {file.status}
                        </Text>
                      </View>
                      <Text style={styles.fileTotals}>
                        <Text style={{ color: t.success }}>+{file.additions}</Text>
                        <Text style={{ color: t.mutedForeground }}> </Text>
                        <Text style={{ color: t.error }}>−{file.deletions}</Text>
                      </Text>
                    </Pressable>

                    {expanded ? (
                      <View style={styles.diffBody}>
                        {diffLoadingPath === file.path ? (
                          <ActivityIndicator size="small" color={t.mutedForeground} />
                        ) : diffError && !diff ? (
                          <Text style={[styles.error, { color: t.error }]}>{diffError}</Text>
                        ) : diff?.binary ? (
                          <Text style={[styles.empty, { color: t.mutedForeground }]}>
                            Binary file preview is unavailable.
                          </Text>
                        ) : diff?.patch ? (
                          <>
                            <ToolDiffViewer
                              presentation={{
                                kind: "patch",
                                patch: diff.patch,
                                filePath: file.path,
                                additions: file.additions,
                                deletions: file.deletions,
                              }}
                            />
                            {diff.truncated ? (
                              <Text style={[styles.note, { color: t.mutedForeground }]}>
                                Large diff truncated for this preview.
                              </Text>
                            ) : null}
                          </>
                        ) : null}
                      </View>
                    ) : null}
                  </View>
                );
              })}
            </View>
          )}
        </>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    padding: 18,
    paddingBottom: 32,
    gap: 14,
  },
  headingRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  headingCopy: {
    flex: 1,
  },
  title: {
    fontSize: 19,
    fontWeight: "700",
  },
  subtitle: {
    marginTop: 4,
    fontSize: 12,
    lineHeight: 17,
  },
  refreshButton: {
    width: 38,
    height: 38,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  branchRow: {
    minHeight: 54,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingBottom: 12,
  },
  branchCopy: {
    flex: 1,
  },
  branch: {
    fontSize: 14,
    fontWeight: "700",
  },
  detail: {
    marginTop: 3,
    fontSize: 12,
  },
  diffTotals: {
    fontSize: 12,
    fontWeight: "700",
    fontVariant: ["tabular-nums"],
  },
  fileList: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    overflow: "hidden",
  },
  fileItem: {
    minWidth: 0,
  },
  fileRow: {
    minHeight: 58,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 9,
  },
  fileCopy: {
    flex: 1,
    minWidth: 0,
  },
  filePath: {
    fontSize: 12,
    fontWeight: "600",
  },
  fileStatus: {
    marginTop: 3,
    fontSize: 11,
    textTransform: "capitalize",
  },
  fileTotals: {
    fontSize: 11,
    fontWeight: "700",
    fontVariant: ["tabular-nums"],
  },
  diffBody: {
    paddingHorizontal: 8,
    paddingBottom: 10,
    gap: 8,
  },
  note: {
    fontSize: 11,
  },
  empty: {
    fontSize: 13,
    lineHeight: 19,
  },
  error: {
    fontSize: 13,
    lineHeight: 19,
  },
});
