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
const CHAT_SESSION_CATEGORY = "CHAT_SESSION";
const MAKO_SESSION_CATEGORY = "MAKO_SESSION";
type NotificationResponseData = {
  requestId?: string;
  sessionId?: string;
  focus?: string;
  messageId?: string;
  reportId?: string;
};
type NotificationResponseEvent = {
  actionIdentifier: string;
  notification: {
    request: {
      identifier?: string;
      content: {
        data?: unknown;
      };
    };
  };
};

function notificationResponseData(value: unknown): NotificationResponseData {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  const root = value as Record<string, unknown>;
  const nested =
    root.data && typeof root.data === "object" && !Array.isArray(root.data)
      ? (root.data as Record<string, unknown>)
      : {};
  return { ...nested, ...root } as NotificationResponseData;
}

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

  await Notifications.setNotificationCategoryAsync(CHAT_SESSION_CATEGORY, [
    {
      identifier: "VIEW_CHAT",
      buttonTitle: "View Chat",
      options: { opensAppToForeground: true },
    },
  ]);

  await Notifications.setNotificationCategoryAsync(MAKO_SESSION_CATEGORY, [
    {
      identifier: "OPEN_MAKO",
      buttonTitle: "Open Mako",
      options: { opensAppToForeground: true },
    },
  ]);
}

type RegisteredNotificationTokens = {
  displayToken: string | null;
  nativeDeviceToken: string | null;
};

async function registerForPushNotifications(): Promise<RegisteredNotificationTokens> {
  if (!Notifications || !Device || !Device.isDevice) {
    return { displayToken: null, nativeDeviceToken: null };
  }

  const { status: existing } = await Notifications.getPermissionsAsync();
  let finalStatus = existing;

  if (existing !== "granted") {
    const { status } = await Notifications.requestPermissionsAsync();
    finalStatus = status;
  }

  if (finalStatus !== "granted") {
    return { displayToken: null, nativeDeviceToken: null };
  }

  const tokenData = await Notifications.getExpoPushTokenAsync({
    projectId: "6e327449-af3c-4138-b1c4-7ceca2baf243",
  }).catch(() => null);
  const nativeTokenData =
    Platform.OS === "ios"
      ? await Notifications.getDevicePushTokenAsync().catch(() => null)
      : null;

  return {
    displayToken:
      (typeof tokenData?.data === "string" && tokenData.data) ||
      (typeof nativeTokenData?.data === "string" ? nativeTokenData.data : null),
    nativeDeviceToken:
      typeof nativeTokenData?.data === "string" ? nativeTokenData.data : null,
  };
}

interface UseNotificationsOptions {
  serverUrl?: string;
  onToolApproval?: (
    sessionId: string,
    requestId: string,
    approved: boolean,
  ) => void;
  onNavigate?: (route: string, params?: Record<string, string>) => void;
  onRegisterNativeDevice?: (deviceToken: string) => boolean | Promise<boolean>;
}

export function useNotifications(options?: UseNotificationsOptions) {
  const [pushToken, setPushToken] = useState<string | null>(null);
  const [nativeDeviceToken, setNativeDeviceToken] = useState<string | null>(null);
  const [notificationLevel, setNotificationLevel] =
    useState<NotificationLevel>("important");
  const responseListenerRef = useRef<any>(null);
  const handledResponseIdRef = useRef<string | null>(null);
  const nativeDeliveryRegisteredRef = useRef(false);
  const optionsRef = useRef(options);
  optionsRef.current = options;

  useEffect(() => {
    if (!Notifications) return;
    void registerNotificationCategories().catch(() => {});

    SecureStore.getItemAsync(NOTIFICATION_LEVEL_KEY).then(
      (saved: string | null) => {
        if (saved === "all" || saved === "important" || saved === "silent") {
          setNotificationLevel(saved);
        }
      },
    );

    registerForPushNotifications().then(
      async ({ displayToken, nativeDeviceToken: token }) => {
        if (displayToken) {
          setPushToken(displayToken);
          await SecureStore.setItemAsync(PUSH_TOKEN_KEY, displayToken);
        }
        if (token) setNativeDeviceToken(token);
      },
    );

    const handleResponse = (response: NotificationResponseEvent) => {
      const responseId = response.notification.request.identifier;
      if (responseId && handledResponseIdRef.current === responseId) {
        return;
      }
      if (responseId) {
        handledResponseIdRef.current = responseId;
      }
      const actionId = response.actionIdentifier;
      const data = notificationResponseData(
        response.notification.request.content.data,
      );

      if (actionId === "APPROVE" && data.requestId && data.sessionId) {
        optionsRef.current?.onToolApproval?.(data.sessionId, data.requestId, true);
      } else if (actionId === "DENY" && data.requestId && data.sessionId) {
        optionsRef.current?.onToolApproval?.(data.sessionId, data.requestId, false);
      } else if (actionId === "VIEW_CHAT" && data.sessionId) {
        optionsRef.current?.onNavigate?.("/(tabs)", { sessionId: data.sessionId });
      } else if (actionId === "OPEN_MAKO") {
        const params: Record<string, string> = { focus: "mako" };
        if (data.sessionId) {
          params.sessionId = data.sessionId;
        }
        if (data.messageId) {
          params.messageId = data.messageId;
        }
        if (data.reportId) {
          params.reportId = data.reportId;
        }
        optionsRef.current?.onNavigate?.("/(tabs)", params);
      } else if (
        actionId === Notifications?.DEFAULT_ACTION_IDENTIFIER &&
        (data.sessionId || data.focus === "mako")
      ) {
        const params: Record<string, string> = {};
        if (data.sessionId) {
          params.sessionId = data.sessionId;
        }
        if (data.focus) {
          params.focus = data.focus;
        }
        if (data.messageId) {
          params.messageId = data.messageId;
        }
        if (data.reportId) {
          params.reportId = data.reportId;
        }
        optionsRef.current?.onNavigate?.("/(tabs)", params);
      }

      if (responseId) {
        void Notifications.dismissNotificationAsync(responseId).catch(() => {});
      }
    };

    responseListenerRef.current =
      Notifications.addNotificationResponseReceivedListener(handleResponse);
    void Notifications.getLastNotificationResponseAsync()
      .then((response: NotificationResponseEvent | null) => {
        if (response) {
          handleResponse(response);
          return Notifications.clearLastNotificationResponseAsync();
        }
      })
      .catch(() => {});

    return () => {
      responseListenerRef.current?.remove();
    };
  }, []);

  useEffect(() => {
    if (!nativeDeviceToken || !options?.onRegisterNativeDevice) return;
    let cancelled = false;

    void Promise.resolve()
      .then(() => options.onRegisterNativeDevice?.(nativeDeviceToken))
      .then((registered) => {
        if (!cancelled) {
          nativeDeliveryRegisteredRef.current = registered === true;
        }
      })
      .catch(() => {
        if (!cancelled) nativeDeliveryRegisteredRef.current = false;
      });

    return () => {
      cancelled = true;
    };
  }, [nativeDeviceToken, options?.onRegisterNativeDevice]);

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
      if (Platform.OS === "ios" && nativeDeliveryRegisteredRef.current) return;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: "Permission Required",
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
      if (Platform.OS === "ios" && nativeDeliveryRegisteredRef.current) return;

      const m = Math.floor(elapsedSeconds / 60);
      const s = elapsedSeconds % 60;
      const timeStr = m > 0 ? `${m}m ${s}s` : `${s}s`;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: `${chatTitle || "Chat"} — Complete`,
          body: `Response finished in ${timeStr} (${tokenCount.toLocaleString()} tokens)`,
          categoryIdentifier: CHAT_SESSION_CATEGORY,
          data: { sessionId, type: "chat_update", kind: "completion", focus: "chat" },
          sound: false,
        },
        trigger: null,
      });
    },
    [notificationLevel],
  );

  const notifyMakoUpdate = useCallback(
    async (title: string, body: string, sessionId?: string) => {
      if (!Notifications || notificationLevel === "silent") return;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: `Mako: ${title}`,
          body,
          categoryIdentifier: MAKO_SESSION_CATEGORY,
          data: { type: "mako_update", kind: "user_message", sessionId, focus: "mako" },
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
