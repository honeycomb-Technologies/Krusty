import { useEffect, useRef, useCallback, useState } from "react";
import { AppState, Platform } from "react-native";
import * as SecureStore from "../platform/secure-store";

export type NotificationLevel = "all" | "important" | "silent";

// Native-only — expo-notifications and expo-device crash on web
let Notifications: any = null;
let Device: any = null;

if (Platform.OS !== "web") {
  try {
    Notifications = require("expo-notifications");
    Device = require("expo-device");
    Notifications.setNotificationHandler({
      handleNotification: async () => ({
        shouldShowAlert: false,
        shouldShowBanner: false,
        shouldShowList: false,
        shouldPlaySound: false,
        shouldSetBadge: false,
      }),
    });
  } catch {
    // Not available
  }
}

const PUSH_TOKEN_KEY = "krusty_push_token";
const NOTIFICATION_LEVEL_KEY = "krusty_notification_level";

const TOOL_APPROVAL_CATEGORY = "TOOL_APPROVAL";
const STREAM_COMPLETE_CATEGORY = "STREAM_COMPLETE";
const MAKO_UPDATE_CATEGORY = "MAKO_UPDATE";

async function registerNotificationCategories() {
  if (!Notifications) return;
  await Notifications.setNotificationCategoryAsync(TOOL_APPROVAL_CATEGORY, [
    {
      identifier: "APPROVE",
      buttonTitle: "Approve",
      options: { opensAppToForeground: false },
    },
    {
      identifier: "DENY",
      buttonTitle: "Deny",
      options: { opensAppToForeground: false, isDestructive: true },
    },
  ]);

  await Notifications.setNotificationCategoryAsync(STREAM_COMPLETE_CATEGORY, [
    {
      identifier: "VIEW",
      buttonTitle: "View Chat",
      options: { opensAppToForeground: true },
    },
  ]);

  await Notifications.setNotificationCategoryAsync(MAKO_UPDATE_CATEGORY, [
    {
      identifier: "VIEW_REPORT",
      buttonTitle: "View Report",
      options: { opensAppToForeground: true },
    },
  ]);
}

async function registerForPushNotifications(): Promise<string | null> {
  if (!Notifications || !Device || !Device.isDevice) return null;

  const { status: existing } = await Notifications.getPermissionsAsync();
  let finalStatus = existing;

  if (existing !== "granted") {
    const { status } = await Notifications.requestPermissionsAsync();
    finalStatus = status;
  }

  if (finalStatus !== "granted") return null;

  const tokenData = await Notifications.getExpoPushTokenAsync({
    projectId: "6e327449-af3c-4138-b1c4-7ceca2baf243",
  });

  return tokenData.data;
}

interface UseNotificationsOptions {
  serverUrl?: string;
  onToolApproval?: (
    sessionId: string,
    requestId: string,
    approved: boolean,
  ) => void;
  onNavigate?: (route: string, params?: Record<string, string>) => void;
}

export function useNotifications(options?: UseNotificationsOptions) {
  const [pushToken, setPushToken] = useState<string | null>(null);
  const [notificationLevel, setNotificationLevel] =
    useState<NotificationLevel>("important");
  const responseListenerRef = useRef<any>(null);

  useEffect(() => {
    if (!Notifications) return;
    registerNotificationCategories();

    SecureStore.getItemAsync(NOTIFICATION_LEVEL_KEY).then(
      (saved: string | null) => {
        if (saved === "all" || saved === "important" || saved === "silent") {
          setNotificationLevel(saved);
        }
      },
    );

    registerForPushNotifications().then(async (token) => {
      if (token) {
        setPushToken(token);
        await SecureStore.setItemAsync(PUSH_TOKEN_KEY, token);
      }
    });

    responseListenerRef.current =
      Notifications.addNotificationResponseReceivedListener((response) => {
        const actionId = response.actionIdentifier;
        const data = response.notification.request.content.data as Record<
          string,
          string
        >;

        if (actionId === "APPROVE" && data.requestId && data.sessionId) {
          options?.onToolApproval?.(data.sessionId, data.requestId, true);
        } else if (actionId === "DENY" && data.requestId && data.sessionId) {
          options?.onToolApproval?.(data.sessionId, data.requestId, false);
        } else if (actionId === "VIEW" && data.sessionId) {
          options?.onNavigate?.("/(tabs)", { sessionId: data.sessionId });
        } else if (actionId === "VIEW_REPORT") {
          options?.onNavigate?.("/(tabs)", { openReports: "true" });
        } else if (
          actionId === Notifications?.DEFAULT_ACTION_IDENTIFIER &&
          data.sessionId
        ) {
          options?.onNavigate?.("/(tabs)", { sessionId: data.sessionId });
        }
      });

    return () => {
      responseListenerRef.current?.remove();
    };
  }, [options?.onNavigate, options?.onToolApproval]);

  const changeNotificationLevel = useCallback(
    async (level: NotificationLevel) => {
      setNotificationLevel(level);
      await SecureStore.setItemAsync(NOTIFICATION_LEVEL_KEY, level);
    },
    [],
  );

  const notifyToolApproval = useCallback(
    async (requestId: string, toolName: string, sessionId: string) => {
      if (!Notifications || notificationLevel === "silent") return;
      if (AppState.currentState === "active") return;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: "Tool Approval Required",
          body: `"${toolName}" is requesting permission to execute.`,
          categoryIdentifier: TOOL_APPROVAL_CATEGORY,
          data: { requestId, sessionId, type: "tool_approval" },
          sound: "default",
        },
        trigger: null,
      });
    },
    [notificationLevel],
  );

  const notifyStreamComplete = useCallback(
    async (
      sessionId: string,
      chatTitle: string,
      tokenCount: number,
      elapsedSeconds: number,
    ) => {
      if (!Notifications || notificationLevel === "silent") return;
      if (AppState.currentState === "active") return;

      const m = Math.floor(elapsedSeconds / 60);
      const s = elapsedSeconds % 60;
      const timeStr = m > 0 ? `${m}m ${s}s` : `${s}s`;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: `${chatTitle || "Chat"} — Complete`,
          body: `Response finished in ${timeStr} (${tokenCount.toLocaleString()} tokens)`,
          categoryIdentifier: STREAM_COMPLETE_CATEGORY,
          data: { sessionId, type: "stream_complete" },
          sound: false,
        },
        trigger: null,
      });
    },
    [notificationLevel],
  );

  const notifyMakoUpdate = useCallback(
    async (title: string, body: string) => {
      if (!Notifications || notificationLevel === "silent") return;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: `Mako: ${title}`,
          body,
          categoryIdentifier: MAKO_UPDATE_CATEGORY,
          data: { type: "mako_update" },
          sound: notificationLevel === "all" ? "default" : false,
        },
        trigger: null,
      });
    },
    [notificationLevel],
  );

  return {
    pushToken,
    notificationLevel,
    changeNotificationLevel,
    notifyToolApproval,
    notifyStreamComplete,
    notifyMakoUpdate,
  };
}
