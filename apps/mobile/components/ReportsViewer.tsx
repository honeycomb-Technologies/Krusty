import { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  Pressable,
  FlatList,
  ScrollView,
  StyleSheet,
  Dimensions,
  ActivityIndicator,
} from 'react-native';
import { BlurView } from '../platform/blur';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withTiming,
  interpolate,
  Easing,
} from 'react-native-reanimated';
import { ArrowLeft, X } from 'lucide-react-native';
import * as Haptics from '../platform/haptics';
import { useThemeContext } from '../hooks/useTheme';
import { useConnection } from '../hooks/useConnection';
import { ReportDetailContent } from './reports/ReportDetailContent';
import type { ReportSummary, Report } from '@krusty/api';

const SCREEN_HEIGHT = Dimensions.get('window').height;
const PANEL_HEIGHT = SCREEN_HEIGHT * 0.88;
const SPRING = { damping: 22, stiffness: 280, mass: 0.8 };

interface ReportsViewerProps {
  visible: boolean;
  onClose: () => void;
}

interface ReportsContentProps {
  visible: boolean;
}

function formatRelativeTime(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(dateStr).toLocaleDateString();
}

export function ReportsViewer({ visible, onClose }: ReportsViewerProps) {
  const { theme } = useThemeContext();
  const { client } = useConnection();
  const t = theme.colors;

  const progress = useSharedValue(0);
  const [mounted, setMounted] = useState(false);

  const [reports, setReports] = useState<ReportSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedReport, setSelectedReport] = useState<Report | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  // Animate in/out
  useEffect(() => {
    if (visible) {
      setMounted(true);
      progress.value = withSpring(1, SPRING);
    } else {
      progress.value = withTiming(0, { duration: 200, easing: Easing.out(Easing.cubic) });
      const timer = setTimeout(() => {
        setMounted(false);
        setSelectedReport(null);
      }, 220);
      return () => clearTimeout(timer);
    }
  }, [visible]);

  // Fetch reports when visible
  useEffect(() => {
    if (visible && client) {
      setLoading(true);
      client
        .getReports()
        .then((res) => setReports(res.reports))
        .catch(() => {})
        .finally(() => setLoading(false));
    }
  }, [visible, client]);

  const handleSelectReport = useCallback(
    async (id: string) => {
      if (!client) return;
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      setDetailLoading(true);
      try {
        const report = await client.getReport(id);
        setSelectedReport(report);
      } catch {
        // silent
      } finally {
        setDetailLoading(false);
      }
    },
    [client],
  );

  const handleBack = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setSelectedReport(null);
  }, []);

  const panelStyle = useAnimatedStyle(() => ({
    transform: [
      { translateY: interpolate(progress.value, [0, 1], [PANEL_HEIGHT, 0]) },
    ],
    opacity: progress.value,
  }));

  const backdropStyle = useAnimatedStyle(() => ({
    opacity: interpolate(progress.value, [0, 1], [0, 1]),
    pointerEvents: progress.value > 0.1 ? ('auto' as const) : ('none' as const),
  }));

  if (!mounted) return null;

  const renderReportCard = ({ item }: { item: ReportSummary }) => (
    <Pressable
      onPress={() => handleSelectReport(item.id)}
      style={[styles.card, { borderColor: t.border }]}
    >
      <Text style={[styles.cardTitle, { color: t.foreground }]} numberOfLines={2}>
        {item.title}
      </Text>
      {item.summary ? (
        <Text
          style={[styles.cardSummary, { color: t.mutedForeground }]}
          numberOfLines={3}
        >
          {item.summary}
        </Text>
      ) : null}
      <View style={styles.cardFooter}>
        <View style={styles.tagsRow}>
          {item.tags.slice(0, 4).map((tag) => (
            <View
              key={tag}
              style={[styles.tagPill, { backgroundColor: t.userMessage + '18', borderColor: t.userMessage + '30' }]}
            >
              <Text style={[styles.tagText, { color: t.userMessage }]}>{tag}</Text>
            </View>
          ))}
        </View>
        <Text style={[styles.cardDate, { color: t.mutedForeground }]}>
          {formatRelativeTime(item.created_at)}
        </Text>
      </View>
    </Pressable>
  );

  return (
    <>
      <Animated.View style={[styles.backdrop, backdropStyle]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} />
      </Animated.View>

      <Animated.View
        style={[styles.panel, { height: PANEL_HEIGHT }, panelStyle]}
      >
        <BlurView
          intensity={50}
          tint={theme.scheme === 'dark' ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight'}
          style={StyleSheet.absoluteFill}
        />
        <View
          style={[
            StyleSheet.absoluteFill,
            {
              backgroundColor:
                theme.scheme === 'dark'
                  ? 'rgba(11,17,25,0.92)'
                  : 'rgba(255,255,255,0.92)',
            },
          ]}
        />

        {/* Header */}
        <View style={[styles.header, { borderBottomColor: t.border }]}>
          {selectedReport ? (
            <Pressable onPress={handleBack} style={styles.headerBtn}>
              <ArrowLeft size={22} color={t.foreground} strokeWidth={1.8} />
            </Pressable>
          ) : (
            <View style={styles.headerBtn} />
          )}
          <Text style={[styles.headerTitle, { color: t.foreground }]} numberOfLines={1}>
            {selectedReport ? selectedReport.title : 'Papers'}
          </Text>
          <Pressable onPress={onClose} style={styles.headerBtn}>
            <X size={20} color={t.mutedForeground} strokeWidth={1.8} />
          </Pressable>
        </View>

        {/* Content */}
        {selectedReport ? (
          <ScrollView
            style={styles.detailScroll}
            contentContainerStyle={styles.detailContent}
            showsVerticalScrollIndicator={false}
          >
            <ReportDetailContent report={selectedReport} />
          </ScrollView>
        ) : loading ? (
          <View style={styles.centered}>
            <ActivityIndicator color={t.mutedForeground} />
          </View>
        ) : reports.length === 0 ? (
          <View style={styles.centered}>
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              No papers yet
            </Text>
          </View>
        ) : (
          <FlatList
            data={reports}
            keyExtractor={(item) => item.id}
            renderItem={renderReportCard}
            contentContainerStyle={styles.listContent}
            showsVerticalScrollIndicator={false}
          />
        )}

        {detailLoading && (
          <View style={styles.detailOverlay}>
            <ActivityIndicator color={t.mutedForeground} />
          </View>
        )}
      </Animated.View>
    </>
  );
}

export function ReportsContent({ visible }: ReportsContentProps) {
  const { theme } = useThemeContext();
  const { client } = useConnection();
  const t = theme.colors;

  const [reports, setReports] = useState<ReportSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedReport, setSelectedReport] = useState<Report | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  useEffect(() => {
    if (visible && client) {
      setLoading(true);
      client
        .getReports()
        .then((res) => setReports(res.reports))
        .catch(() => {})
        .finally(() => setLoading(false));
    }
  }, [visible, client]);

  const handleSelectReport = useCallback(
    async (id: string) => {
      if (!client) return;
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      setDetailLoading(true);
      try {
        const report = await client.getReport(id);
        setSelectedReport(report);
      } catch {
        // silent
      } finally {
        setDetailLoading(false);
      }
    },
    [client],
  );

  const handleBack = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setSelectedReport(null);
  }, []);

  const renderReportCard = ({ item }: { item: ReportSummary }) => (
    <Pressable
      onPress={() => handleSelectReport(item.id)}
      style={[styles.card, { borderColor: t.border }]}
    >
      <Text style={[styles.cardTitle, { color: t.foreground }]} numberOfLines={2}>
        {item.title}
      </Text>
      {item.summary ? (
        <Text
          style={[styles.cardSummary, { color: t.mutedForeground }]}
          numberOfLines={3}
        >
          {item.summary}
        </Text>
      ) : null}
      <View style={styles.cardFooter}>
        <View style={styles.tagsRow}>
          {item.tags.slice(0, 4).map((tag) => (
            <View
              key={tag}
              style={[styles.tagPill, { backgroundColor: t.userMessage + '18', borderColor: t.userMessage + '30' }]}
            >
              <Text style={[styles.tagText, { color: t.userMessage }]}>{tag}</Text>
            </View>
          ))}
        </View>
        <Text style={[styles.cardDate, { color: t.mutedForeground }]}>
          {formatRelativeTime(item.created_at)}
        </Text>
      </View>
    </Pressable>
  );

  if (!visible) return null;

  return (
    <View style={styles.reportsContentContainer}>
      {selectedReport ? (
        <>
          <View style={[styles.contentHeader, { borderBottomColor: t.border }]}>
            <Pressable onPress={handleBack} style={styles.headerBtn}>
              <ArrowLeft size={22} color={t.foreground} strokeWidth={1.8} />
            </Pressable>
            <Text style={[styles.headerTitle, { color: t.foreground }]} numberOfLines={1}>
              {selectedReport.title}
            </Text>
            <View style={styles.headerBtn} />
          </View>
          <ScrollView
            style={styles.detailScroll}
            contentContainerStyle={styles.detailContent}
            showsVerticalScrollIndicator={false}
          >
            <ReportDetailContent report={selectedReport} />
          </ScrollView>
        </>
      ) : loading ? (
        <View style={styles.centered}>
          <ActivityIndicator color={t.mutedForeground} />
        </View>
      ) : reports.length === 0 ? (
        <View style={styles.centered}>
          <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
            No papers yet
          </Text>
        </View>
      ) : (
        <FlatList
          data={reports}
          keyExtractor={(item) => item.id}
          renderItem={renderReportCard}
          contentContainerStyle={styles.listContent}
          showsVerticalScrollIndicator={false}
        />
      )}

      {detailLoading && (
        <View style={styles.detailOverlay}>
          <ActivityIndicator color={t.mutedForeground} />
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.5)',
    zIndex: 200,
  },
  panel: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 201,
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 12,
  },
  headerBtn: {
    width: 32,
    alignItems: 'center',
  },
  headerTitle: {
    flex: 1,
    fontSize: 17,
    fontWeight: '700',
    textAlign: 'center',
  },
  listContent: {
    padding: 16,
    paddingBottom: 40,
  },
  card: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    padding: 14,
    marginBottom: 12,
  },
  cardTitle: {
    fontSize: 16,
    fontWeight: '600',
    lineHeight: 21,
    marginBottom: 6,
  },
  cardSummary: {
    fontSize: 14,
    lineHeight: 19,
    marginBottom: 10,
  },
  cardFooter: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  tagsRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 6,
    flex: 1,
  },
  tagPill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
  },
  tagText: {
    fontSize: 11,
    fontWeight: '600',
  },
  cardDate: {
    fontSize: 12,
    marginLeft: 8,
  },
  centered: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
  },
  emptyText: {
    fontSize: 16,
  },
  detailScroll: {
    flex: 1,
  },
  detailContent: {
    padding: 16,
    paddingBottom: 40,
  },
  detailOverlay: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.3)',
    justifyContent: 'center',
    alignItems: 'center',
  },
  reportsContentContainer: {
    flex: 1,
  },
  contentHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 12,
  },
});
