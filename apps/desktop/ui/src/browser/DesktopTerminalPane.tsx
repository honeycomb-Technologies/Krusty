import { useMemo, useState } from 'react';
import {
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { Copy, ExternalLink } from 'lucide-react-native';
import { useThemeContext } from '@mobile/hooks/useTheme';
import { Terminal } from '@mobile/components/desktop/Terminal';
import * as Clipboard from '@mobile/platform/clipboard';
import {
  buildGhosttyOpenCommand,
  openGhostty,
} from '../host/desktopHost';

export function DesktopTerminalPane({
  visible,
  projectDirectory,
}: {
  visible: boolean;
  projectDirectory?: string | null;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [showEmbedded, setShowEmbedded] = useState(false);
  const [copied, setCopied] = useState(false);

  const command = useMemo(
    () => buildGhosttyOpenCommand(projectDirectory),
    [projectDirectory],
  );

  if (!visible) return null;

  const workspaceLabel = projectDirectory
    ? projectDirectory.split('/').filter(Boolean).slice(-2).join('/')
    : 'No project';

  const handleOpenGhostty = async () => {
    setBusy(true);
    setStatus(null);
    const result = await openGhostty(projectDirectory);
    setStatus(result.ok ? null : result.message);
    setBusy(false);
  };

  const handleCopy = async () => {
    await Clipboard.setStringAsync(command);
    setCopied(true);
    setTimeout(() => setCopied(false), 1000);
  };

  return (
    <View style={[styles.root, { backgroundColor: t.background }]}>
      <View style={styles.body}>
        {projectDirectory ? (
          <Text style={[styles.meta, { color: t.mutedForeground }]} numberOfLines={1}>
            {workspaceLabel}
          </Text>
        ) : null}
        <Pressable
          onPress={() => void handleOpenGhostty()}
          disabled={busy}
          style={[
            styles.primaryBtn,
            {
              backgroundColor: t.userMessage,
              opacity: busy ? 0.6 : 1,
            },
          ]}
        >
          <ExternalLink size={15} color="#fff" />
          <Text style={styles.primaryBtnText}>
            {busy ? 'Opening…' : projectDirectory ? 'Open here' : 'Open Ghostty'}
          </Text>
        </Pressable>

        <Pressable
          onPress={() => void handleCopy()}
          style={[styles.secondaryBtn, { borderColor: t.border }]}
        >
          <Copy size={14} color={t.mutedForeground} />
          <Text style={{ color: t.foreground, fontWeight: '700', fontSize: 12 }}>
            {copied ? 'Copied' : 'Copy command'}
          </Text>
        </Pressable>

        {status ? (
          <Text style={[styles.status, { color: t.warning }]}>{status}</Text>
        ) : null}

        <Pressable onPress={() => setShowEmbedded((value) => !value)} hitSlop={8}>
          <Text style={{ color: t.mutedForeground, fontSize: 11 }}>
            {showEmbedded ? 'Hide embedded Ghostty' : 'Embedded Ghostty'}
          </Text>
        </Pressable>

        {showEmbedded ? (
          <View style={[styles.embedded, { borderColor: t.border }]}>
            <Terminal visible style={styles.embeddedTerminal} />
          </View>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  meta: { fontSize: 12, marginBottom: 2 },
  body: {
    flex: 1,
    padding: 14,
    gap: 10,
  },
  primaryBtn: {
    minHeight: 40,
    borderRadius: 10,
    paddingHorizontal: 12,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 8,
  },
  primaryBtnText: { color: '#fff', fontWeight: '800', fontSize: 13 },
  secondaryBtn: {
    minHeight: 34,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 10,
    flexDirection: 'row',
    gap: 8,
  },
  status: { fontSize: 12, lineHeight: 17 },
  embedded: {
    flex: 1,
    minHeight: 220,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    overflow: 'hidden',
  },
  embeddedTerminal: { flex: 1, height: undefined, borderTopWidth: 0 },
});
