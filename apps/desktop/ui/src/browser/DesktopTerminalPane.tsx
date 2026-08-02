import { useMemo, useState } from 'react';
import {
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { Copy, ExternalLink, RefreshCw } from 'lucide-react-native';
import { useConnection } from '@mobile/hooks/useConnection';
import { useThemeContext } from '@mobile/hooks/useTheme';
import { buildTerminalWebSocketUrl } from '@mobile/components/terminalUrl';
import { getTerminalHtml } from '@mobile/components/toolbox/terminalHtml';
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
  const { serverUrl, serverToken, isConnected } = useConnection();
  const t = theme.colors;
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [showEmbedded, setShowEmbedded] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [copied, setCopied] = useState(false);

  const command = useMemo(
    () => buildGhosttyOpenCommand(projectDirectory),
    [projectDirectory],
  );

  const wsUrl = useMemo(() => {
    if (!serverUrl) return null;
    return buildTerminalWebSocketUrl(serverUrl, serverToken);
  }, [serverToken, serverUrl]);

  const html = useMemo(() => {
    if (!wsUrl) return null;
    return getTerminalHtml(wsUrl, {
      background: t.background,
      foreground: t.foreground,
      cursor: t.userMessage,
    });
  }, [t.background, t.foreground, t.userMessage, wsUrl]);

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
            {showEmbedded ? 'Hide fallback' : 'Fallback'}
          </Text>
        </Pressable>

        {showEmbedded ? (
          <View style={[styles.embedded, { borderColor: t.border }]}>
            <View style={[styles.embeddedHeader, { borderBottomColor: t.border }]}>
              <Text style={{ color: t.mutedForeground, fontSize: 11, fontWeight: '700' }}>
                FALLBACK
              </Text>
              <Pressable onPress={() => setReloadKey((value) => value + 1)}>
                <RefreshCw size={13} color={t.mutedForeground} />
              </Pressable>
            </View>
            <View style={styles.embeddedStage}>
              {!isConnected || !html ? (
                <Text style={{ color: t.mutedForeground, fontSize: 12 }}>
                  Connect a server to use fallback terminal.
                </Text>
              ) : Platform.OS === 'web' ? (
                // @ts-expect-error iframe is web-only
                <iframe
                  key={reloadKey}
                  title="Embedded terminal fallback"
                  srcDoc={html}
                  style={{
                    border: '0',
                    width: '100%',
                    height: '100%',
                    background: t.background,
                  }}
                  sandbox="allow-scripts allow-same-origin"
                />
              ) : (
                <Text style={{ color: t.mutedForeground }}>Fallback is web-only.</Text>
              )}
            </View>
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
  embeddedHeader: {
    minHeight: 32,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  embeddedStage: { flex: 1 },
});
