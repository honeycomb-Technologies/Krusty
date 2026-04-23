import { View, Text } from "react-native";
import { Bell, Moon } from "lucide-react-native";

import type {
  OAuthStartResponse,
  PortEntry,
  ProviderStatus,
} from "@krusty/api";
import type { ColorScheme } from "@krusty/ui";

import { useNotifications, type NotificationLevel } from "../../hooks/useNotifications";
import { useThemeContext } from "../../hooks/useTheme";
import { styles } from "./styles";

export type ProviderFormState = Record<string, string>;

export type PreviewDraftState = {
  autoRefreshSecs: string;
  probeTimeoutMs: string;
};

export interface ActiveOAuthFlow {
  provider: string;
  flowType: OAuthStartResponse["flow_type"];
  authUrl: string;
  pasteCode: boolean;
  userCode?: string | null;
  verificationUriComplete?: string | null;
}

export type SchemeOption = {
  key: ColorScheme;
  label: string;
  icon: typeof Moon;
};

export type NotificationOption = {
  key: NotificationLevel;
  label: string;
  icon: typeof Bell;
};

export function toErrorMessage(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

export function SectionTitle({
  title,
  subtitle,
}: {
  title: string;
  subtitle?: string;
}) {
  const { theme } = useThemeContext();
  return (
    <View style={styles.sectionHeader}>
      <Text style={[styles.sectionTitle, { color: theme.colors.foreground }]}> 
        {title}
      </Text>
      {subtitle ? (
        <Text style={[styles.sectionSubtitle, { color: theme.colors.mutedForeground }]}> 
          {subtitle}
        </Text>
      ) : null}
    </View>
  );
}

export function Pill({
  label,
  tone = "neutral",
}: {
  label: string;
  tone?: "neutral" | "success" | "warning" | "error" | "info";
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const palette = {
    neutral: { backgroundColor: `${t.border}35`, color: t.mutedForeground },
    success: { backgroundColor: `${t.success}20`, color: t.success },
    warning: { backgroundColor: `${t.warning}20`, color: t.warning },
    error: { backgroundColor: `${t.error}20`, color: t.error },
    info: { backgroundColor: `${t.userMessage}18`, color: t.userMessage },
  } as const;
  const selected = palette[tone];

  return (
    <View style={[styles.pill, { backgroundColor: selected.backgroundColor }]}> 
      <Text style={[styles.pillText, { color: selected.color }]}>{label}</Text>
    </View>
  );
}

export function MessageBanner({
  text,
  tone = "error",
}: {
  text: string | null;
  tone?: "error" | "info";
}) {
  const { theme } = useThemeContext();
  if (!text) return null;

  const color = tone === "error" ? theme.colors.error : theme.colors.mutedForeground;
  const bg =
    tone === "error"
      ? `${theme.colors.error}12`
      : `${theme.colors.userMessage}12`;

  return (
    <View style={[styles.banner, { backgroundColor: bg, borderColor: `${color}20` }]}> 
      <Text style={[styles.bannerText, { color }]}>{text}</Text>
    </View>
  );
}

export function previewStatusText(port: PortEntry): string {
  if (!port.active) return "Offline";
  if (port.is_previewable_http) {
    return port.last_probe_ms ? `HTTP ready • ${port.last_probe_ms}ms` : "HTTP ready";
  }

  switch (port.probe_status) {
    case "timeout":
      return "Probe timeout";
    case "conn_refused":
      return "Connection refused";
    case "non_http":
      return "Non-HTTP listener";
    default:
      return "Probe failed";
  }
}

export async function pollOAuthUntilDone(
  provider: string,
  getOAuthStatus: (provider: string) => Promise<{ has_token: boolean; flow_active: boolean }>,
) {
  const deadline = Date.now() + 120_000;
  while (Date.now() < deadline) {
    const status = await getOAuthStatus(provider);
    if (status.has_token || !status.flow_active) {
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }

  return getOAuthStatus(provider);
}
