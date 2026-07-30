import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { AppState, Platform } from "react-native";
import { useRouter } from "expo-router";
import * as SecureStore from "../platform/secure-store";
import { useConnection } from "./useConnection";

export type NotificationLevel = "all" | "important" | "silent";
export type NotificationRegistrationState =
  | "unavailable"
  | "permission_required"
  | "token_ready"
  | "registering"
  | "registered"
  | "error";

// Native-only — expo-notifications and expo-device crash on web
let Notifications: any = null;
let Device: any = null;
let foregroundNotificationLevel: NotificationLevel = "important";

if (Platform.OS !== "web") {
  try {
    Notifications = require("expo-notifications");
    Device = require("expo-device");
    Notifications.setNotificationHandler({
      handleNotification: async (notification: unknown) => {
        const data = notificationResponseData(
          (notification as {
            request?: { content?: { data?: unknown } };
          })?.request?.content?.data,
        );
        const kind = data.kind ?? data.type;
        const important =
          data.type === "tool_approval" ||
          kind === "awaiting_input" ||
          kind === "error";
        const show =
          foregroundNotificationLevel === "all" ||
          (foregroundNotificationLevel === "important" && important);
        return {
          shouldShowAlert: show,
          shouldShowBanner: show,
          shouldShowList: show,
          shouldPlaySound:
            show &&
            (foregroundNotificationLevel === "all" || important),
          shouldSetBadge: false,
        };
      },
    });
  } catch {
    // Not available
  }
}

const PUSH_TOKEN_KEY = "krusty_push_token";
const NOTIFICATION_LEVEL_KEY = "krusty_notification_level";
const PENDING_ACTIONS_KEY = "krusty_pending_notification_actions_v1";
const HANDLED_ACTIONS_KEY = "krusty_handled_notification_actions_v1";
const MAX_HANDLED_ACTIONS = 100;
const ACTION_RETRY_BASE_MS = 1_000;
const EXPO_PROJECT_ID = "6e327449-af3c-4138-b1c4-7ceca2baf243";

const TOOL_APPROVAL_CATEGORY = "TOOL_APPROVAL";
const CHAT_SESSION_CATEGORY = "CHAT_SESSION";
const MAKO_SESSION_CATEGORY = "MAKO_SESSION";
type NotificationResponseData = {
  type?: string;
  kind?: string;
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

type PendingNotificationAction = {
  id: string;
  actionIdentifier: string;
  data: NotificationResponseData;
  responseId?: string;
  createdAt: number;
  attempts: number;
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
      buttonTitle: "Open Hive",
      options: { opensAppToForeground: true },
    },
  ]);
}

type RegisteredNotificationTokens = {
  displayToken: string | null;
  nativeDeviceToken: string | null;
  deviceTokenIdentity: string | null;
};

type DevicePushTokenLike = {
  data?: unknown;
  type?: unknown;
};

function devicePushTokenIdentity(
  token: DevicePushTokenLike | null | undefined,
): string | null {
  if (!token || typeof token.data !== "string" || token.data.length === 0) {
    return null;
  }
  const type = typeof token.type === "string" ? token.type : "unknown";
  return `${type}:${token.data}`;
}

function permissionGranted(settings: any): boolean {
  return Boolean(
    settings?.granted ||
      settings?.status === "granted" ||
      settings?.ios?.status === Notifications?.IosAuthorizationStatus?.PROVISIONAL,
  );
}

async function ensureAndroidNotificationChannel() {
  if (!Notifications || Platform.OS !== "android") return;
  await Notifications.setNotificationChannelAsync("default", {
    name: "Mitsuro",
    importance: Notifications.AndroidImportance?.HIGH ?? 4,
    vibrationPattern: [0, 200, 100, 200],
  });
}

async function registerForPushNotifications(
  requestPermission = true,
  devicePushToken?: DevicePushTokenLike,
): Promise<RegisteredNotificationTokens> {
  if (!Notifications || !Device || !Device.isDevice) {
    return {
      displayToken: null,
      nativeDeviceToken: null,
      deviceTokenIdentity: null,
    };
  }

  await ensureAndroidNotificationChannel();
  let settings = await Notifications.getPermissionsAsync();
  if (!permissionGranted(settings) && requestPermission) {
    settings = await Notifications.requestPermissionsAsync();
  }

  if (!permissionGranted(settings)) {
    return {
      displayToken: null,
      nativeDeviceToken: null,
      deviceTokenIdentity: null,
    };
  }

  const nativeTokenData =
    devicePushToken ??
    (await Notifications.getDevicePushTokenAsync().catch(() => null));
  const tokenData = await Notifications.getExpoPushTokenAsync({
    projectId: EXPO_PROJECT_ID,
    ...(nativeTokenData ? { devicePushToken: nativeTokenData } : {}),
  }).catch(() => null);

  return {
    displayToken:
      typeof tokenData?.data === "string" ? tokenData.data : null,
    nativeDeviceToken:
      Platform.OS === "ios" && typeof nativeTokenData?.data === "string"
        ? nativeTokenData.data
        : null,
    deviceTokenIdentity: devicePushTokenIdentity(nativeTokenData),
  };
}

interface NotificationContextValue {
  pushToken: string | null;
  nativeDeviceToken: string | null;
  notificationLevel: NotificationLevel;
  registrationState: NotificationRegistrationState;
  lastRegistrationError: string | null;
  pendingActionCount: number;
  changeNotificationLevel: (level: NotificationLevel) => Promise<void>;
  submitToolApprovalAction: (
    sessionId: string,
    requestId: string,
    approved: boolean,
  ) => Promise<void>;
  notifyToolApproval: (
    requestId: string,
    toolName: string,
    sessionId: string,
  ) => Promise<void>;
  notifyStreamComplete: (
    sessionId: string,
    chatTitle: string,
    tokenCount: number,
    elapsedSeconds: number,
  ) => Promise<void>;
  notifyMakoUpdate: (
    title: string,
    body: string,
    sessionId?: string,
  ) => Promise<void>;
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

function actionIdentity(
  actionIdentifier: string,
  data: NotificationResponseData,
  responseId?: string,
): string {
  return (
    responseId ??
    [
      actionIdentifier,
      data.sessionId ?? "",
      data.requestId ?? "",
      data.messageId ?? "",
      data.reportId ?? "",
    ].join(":")
  );
}

function parseStoredActions(value: string | null): PendingNotificationAction[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is PendingNotificationAction =>
        Boolean(
          item &&
            typeof item === "object" &&
            typeof item.id === "string" &&
            typeof item.actionIdentifier === "string" &&
            item.data &&
            typeof item.data === "object",
        ),
    );
  } catch {
    return [];
  }
}

function parseHandledActions(value: string | null): string[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed)
      ? parsed.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

export function NotificationProvider({ children }: { children: ReactNode }) {
  const router = useRouter();
  const { client, isConfigured } = useConnection();
  const [pushToken, setPushToken] = useState<string | null>(null);
  const [nativeDeviceToken, setNativeDeviceToken] = useState<string | null>(null);
  const [notificationLevel, setNotificationLevel] =
    useState<NotificationLevel>("important");
  const [registrationState, setRegistrationState] =
    useState<NotificationRegistrationState>(
      Notifications ? "permission_required" : "unavailable",
    );
  const [lastRegistrationError, setLastRegistrationError] = useState<string | null>(
    null,
  );
  const [pendingActionCount, setPendingActionCount] = useState(0);
  const responseListenerRef = useRef<any>(null);
  const nativeDeliveryRegisteredRef = useRef(false);
  const pendingActionsRef = useRef<PendingNotificationAction[]>([]);
  const handledActionsRef = useRef<string[]>([]);
  const processingActionsRef = useRef(false);
  const processPendingActionsRef = useRef<() => void>(() => {});
  const actionRetryRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clientRef = useRef(client);
  const registrationGenerationRef = useRef(0);
  const registrationQueueRef = useRef<Promise<void>>(Promise.resolve());
  const deviceTokenIdentityRef = useRef<string | null>(null);
  clientRef.current = client;
  foregroundNotificationLevel = notificationLevel;

  const persistPendingActions = useCallback(async () => {
    setPendingActionCount(pendingActionsRef.current.length);
    await SecureStore.setItemAsync(
      PENDING_ACTIONS_KEY,
      JSON.stringify(pendingActionsRef.current),
    );
  }, []);

  const rememberHandledAction = useCallback(async (id: string) => {
    handledActionsRef.current = [
      ...handledActionsRef.current.filter((value) => value !== id),
      id,
    ].slice(-MAX_HANDLED_ACTIONS);
    await SecureStore.setItemAsync(
      HANDLED_ACTIONS_KEY,
      JSON.stringify(handledActionsRef.current),
    );
  }, []);

  const navigateForAction = useCallback(
    (data: NotificationResponseData) => {
      const params: Record<string, string> = {};
      if (data.sessionId) params.sessionId = data.sessionId;
      if (data.focus) params.focus = data.focus;
      if (data.messageId) params.messageId = data.messageId;
      if (data.reportId) params.reportId = data.reportId;
      router.replace({ pathname: "/(tabs)", params });
    },
    [router],
  );

  const processPendingActions = useCallback(async () => {
    if (processingActionsRef.current) return;
    processingActionsRef.current = true;
    if (actionRetryRef.current) {
      clearTimeout(actionRetryRef.current);
      actionRetryRef.current = null;
    }

    try {
      while (pendingActionsRef.current.length > 0) {
        const action = pendingActionsRef.current[0];
        try {
          if (
            (action.actionIdentifier === "APPROVE" ||
              action.actionIdentifier === "DENY") &&
            action.data.requestId &&
            action.data.sessionId
          ) {
            const activeClient = clientRef.current;
            if (!activeClient) return;
            await activeClient.submitToolApproval(
              action.data.sessionId,
              action.data.requestId,
              action.actionIdentifier === "APPROVE",
              action.id,
            );
            navigateForAction(action.data);
          } else {
            navigateForAction(action.data);
          }

          pendingActionsRef.current.shift();
          await rememberHandledAction(action.id);
          await persistPendingActions();
          if (action.responseId) {
            await Notifications?.dismissNotificationAsync(action.responseId).catch(
              () => {},
            );
          }
          setLastRegistrationError(null);
        } catch (error) {
          action.attempts += 1;
          await persistPendingActions();
          const message =
            error instanceof Error ? error.message : "Notification action failed";
          setLastRegistrationError(message);
          const delay = Math.min(
            ACTION_RETRY_BASE_MS * 2 ** Math.min(action.attempts, 6),
            60_000,
          );
          actionRetryRef.current = setTimeout(() => {
            processPendingActionsRef.current();
          }, delay);
          return;
        }
      }
    } finally {
      processingActionsRef.current = false;
    }
  }, [navigateForAction, persistPendingActions, rememberHandledAction]);
  processPendingActionsRef.current = () => {
    void processPendingActions();
  };

  const enqueueResponse = useCallback(
    async (response: NotificationResponseEvent) => {
      const responseId = response.notification.request.identifier;
      const actionIdentifier = response.actionIdentifier;
      const data = notificationResponseData(
        response.notification.request.content.data,
      );
      if (
        actionIdentifier !== "APPROVE" &&
        actionIdentifier !== "DENY" &&
        actionIdentifier !== "VIEW_CHAT" &&
        actionIdentifier !== "OPEN_MAKO" &&
        actionIdentifier !== Notifications?.DEFAULT_ACTION_IDENTIFIER
      ) {
        return;
      }

      const id = actionIdentity(actionIdentifier, data, responseId);
      if (
        handledActionsRef.current.includes(id) ||
        pendingActionsRef.current.some((action) => action.id === id)
      ) {
        return;
      }
      pendingActionsRef.current.push({
        id,
        actionIdentifier,
        data,
        responseId,
        createdAt: Date.now(),
        attempts: 0,
      });
      await persistPendingActions();
      await Notifications?.clearLastNotificationResponseAsync().catch(() => {});
      void processPendingActions();
    },
    [persistPendingActions, processPendingActions],
  );

  useEffect(() => {
    if (!Notifications) return;
    let disposed = false;
    void registerNotificationCategories().catch(() => {});

    void Promise.all([
      SecureStore.getItemAsync(NOTIFICATION_LEVEL_KEY),
      SecureStore.getItemAsync(PENDING_ACTIONS_KEY),
      SecureStore.getItemAsync(HANDLED_ACTIONS_KEY),
    ]).then(async ([saved, pending, handled]) => {
        if (disposed) return;
        if (saved === "all" || saved === "important" || saved === "silent") {
          setNotificationLevel(saved);
          foregroundNotificationLevel = saved;
        }
        pendingActionsRef.current = parseStoredActions(pending);
        handledActionsRef.current = parseHandledActions(handled);
        setPendingActionCount(pendingActionsRef.current.length);
        void processPendingActions();

        responseListenerRef.current =
          Notifications.addNotificationResponseReceivedListener(
            (response: NotificationResponseEvent) => {
              void enqueueResponse(response);
            },
          );
        const response =
          await Notifications.getLastNotificationResponseAsync().catch(() => null);
        if (!disposed && response) {
          await enqueueResponse(response);
        }
      }).catch(() => {});

    return () => {
      disposed = true;
      responseListenerRef.current?.remove();
      if (actionRetryRef.current) clearTimeout(actionRetryRef.current);
    };
  }, [enqueueResponse, processPendingActions]);

  useEffect(() => {
    if (!Notifications || !isConfigured) return;
    let cancelled = false;
    let refreshInFlight: Promise<void> | null = null;
    let queuedDevicePushToken: DevicePushTokenLike | undefined;

    const runTokenRefresh = async (devicePushToken?: DevicePushTokenLike) => {
      try {
        const tokens = await registerForPushNotifications(
          true,
          devicePushToken,
        );
        if (cancelled) return;
        deviceTokenIdentityRef.current = tokens.deviceTokenIdentity;
        if (!tokens.displayToken && !tokens.nativeDeviceToken) {
          setRegistrationState("permission_required");
          return;
        }
        setPushToken(tokens.displayToken);
        setNativeDeviceToken(tokens.nativeDeviceToken);
        setRegistrationState("token_ready");
        if (tokens.displayToken) {
          await SecureStore.setItemAsync(PUSH_TOKEN_KEY, tokens.displayToken);
        }
      } catch (error) {
        if (!cancelled) {
          setRegistrationState("error");
          setLastRegistrationError(
            error instanceof Error ? error.message : "Push registration failed",
          );
        }
      }
    };

    const refreshTokens = (
      devicePushToken?: DevicePushTokenLike,
    ): Promise<void> => {
      const identity = devicePushTokenIdentity(devicePushToken);
      if (identity && identity === deviceTokenIdentityRef.current) {
        return Promise.resolve();
      }
      if (refreshInFlight) {
        queuedDevicePushToken = devicePushToken;
        return refreshInFlight;
      }

      const currentRefresh = (async () => {
        let nextDevicePushToken = devicePushToken;
        do {
          queuedDevicePushToken = undefined;
          await runTokenRefresh(nextDevicePushToken);
          nextDevicePushToken = queuedDevicePushToken;
        } while (
          !cancelled &&
          nextDevicePushToken &&
          devicePushTokenIdentity(nextDevicePushToken) !==
            deviceTokenIdentityRef.current
        );
      })();
      refreshInFlight = currentRefresh;
      void currentRefresh.finally(() => {
        if (refreshInFlight !== currentRefresh) return;
        refreshInFlight = null;
        const remainingDevicePushToken = queuedDevicePushToken;
        queuedDevicePushToken = undefined;
        if (
          !cancelled &&
          remainingDevicePushToken &&
          devicePushTokenIdentity(remainingDevicePushToken) !==
            deviceTokenIdentityRef.current
        ) {
          void refreshTokens(remainingDevicePushToken);
        }
      });
      return currentRefresh;
    };

    void refreshTokens();

    const tokenListener = Notifications.addPushTokenListener?.(
      (devicePushToken: DevicePushTokenLike) => {
        void refreshTokens(devicePushToken);
      },
    );
    const appStateListener = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        void refreshTokens();
        processPendingActionsRef.current();
      }
    });

    return () => {
      cancelled = true;
      tokenListener?.remove?.();
      appStateListener.remove();
    };
  }, [isConfigured]);

  useEffect(() => {
    const generation = ++registrationGenerationRef.current;
    if (!client) {
      nativeDeliveryRegisteredRef.current = false;
      if (pushToken || nativeDeviceToken) setRegistrationState("token_ready");
      return;
    }
    const register = async () => {
      if (generation !== registrationGenerationRef.current) return;
      setRegistrationState("registering");
      let directApnsRegistered = false;
      let expoRegistered = false;
      const errors: string[] = [];
      if (Platform.OS === "ios" && nativeDeviceToken) {
        try {
          await client.registerApnsDevice(
            nativeDeviceToken,
            undefined,
            notificationLevel,
            __DEV__ ? "sandbox" : "production",
          );
          if (generation !== registrationGenerationRef.current) return;
          directApnsRegistered = true;
        } catch (error) {
          errors.push(
            error instanceof Error ? error.message : "Direct APNs registration failed",
          );
        }
      }
      if (generation !== registrationGenerationRef.current) return;
      if (pushToken && (Platform.OS !== "ios" || !directApnsRegistered)) {
        try {
          await client.registerExpoPushDevice(
            pushToken,
            Platform.OS === "android" ? "android" : "ios",
            notificationLevel,
          );
          if (generation !== registrationGenerationRef.current) return;
          expoRegistered = true;
        } catch (error) {
          errors.push(
            error instanceof Error ? error.message : "Expo push registration failed",
          );
        }
      }
      if (generation !== registrationGenerationRef.current) return;
      if (pushToken && Platform.OS === "ios" && directApnsRegistered) {
        try {
          await client.unregisterExpoPushDevice(pushToken);
          if (generation !== registrationGenerationRef.current) return;
        } catch (error) {
          errors.push(
            error instanceof Error
              ? error.message
              : "Old Expo push registration could not be removed",
          );
        }
      }
      if (generation !== registrationGenerationRef.current) return;
      const registered = directApnsRegistered || expoRegistered;
      nativeDeliveryRegisteredRef.current = registered;
      if (registered) {
        setRegistrationState("registered");
        setLastRegistrationError(errors.length > 0 ? errors.join(" · ") : null);
        void processPendingActions();
      } else if (errors.length > 0) {
        setRegistrationState("error");
        setLastRegistrationError(errors.join(" · "));
      } else {
        setRegistrationState("token_ready");
        setLastRegistrationError(null);
      }
    };
    registrationQueueRef.current = registrationQueueRef.current
      .catch(() => {})
      .then(register);
  }, [
    client,
    nativeDeviceToken,
    notificationLevel,
    processPendingActions,
    pushToken,
  ]);

  const changeNotificationLevel = useCallback(
    async (level: NotificationLevel) => {
      foregroundNotificationLevel = level;
      setNotificationLevel(level);
      await SecureStore.setItemAsync(NOTIFICATION_LEVEL_KEY, level);
    },
    [],
  );

  const submitToolApprovalAction = useCallback(
    async (sessionId: string, requestId: string, approved: boolean) => {
      const id = `in-app:${sessionId}:${requestId}`;
      if (
        handledActionsRef.current.includes(id) ||
        pendingActionsRef.current.some((action) => action.id === id)
      ) {
        return;
      }
      pendingActionsRef.current.push({
        id,
        actionIdentifier: approved ? "APPROVE" : "DENY",
        data: { sessionId, requestId, type: "tool_approval" },
        createdAt: Date.now(),
        attempts: 0,
      });
      await persistPendingActions();
      await processPendingActions();
    },
    [persistPendingActions, processPendingActions],
  );

  const notifyToolApproval = useCallback(
    async (requestId: string, toolName: string, sessionId: string) => {
      if (!Notifications || notificationLevel === "silent") return;
      if (AppState.currentState === "active") return;
      if (nativeDeliveryRegisteredRef.current) return;

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
      if (nativeDeliveryRegisteredRef.current) return;

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
      if (nativeDeliveryRegisteredRef.current) return;

      await Notifications.scheduleNotificationAsync({
        content: {
          title: `Hive: ${title}`,
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

  const value: NotificationContextValue = {
    pushToken,
    nativeDeviceToken,
    notificationLevel,
    registrationState,
    lastRegistrationError,
    pendingActionCount,
    changeNotificationLevel,
    submitToolApprovalAction,
    notifyToolApproval,
    notifyStreamComplete,
    notifyMakoUpdate,
  };

  return (
    <NotificationContext.Provider value={value}>
      {children}
    </NotificationContext.Provider>
  );
}

export function useNotifications(): NotificationContextValue {
  const context = useContext(NotificationContext);
  if (!context) {
    throw new Error("useNotifications must be used inside NotificationProvider");
  }
  return context;
}
