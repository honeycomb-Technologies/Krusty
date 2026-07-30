declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("push-token changes cannot recursively request the device token", async () => {
  const source = await Deno.readTextFile(
    new URL("../hooks/useNotifications.tsx", import.meta.url).pathname,
  );

  assert(
    source.includes("getExpoPushTokenAsync({") &&
      source.includes("devicePushToken: nativeTokenData"),
    "the Expo token exchange must reuse the already-resolved native token",
  );
  assert(
    source.includes(
      "(devicePushToken: DevicePushTokenLike) => {\n        void refreshTokens(devicePushToken);",
    ),
    "the token listener must pass its token into refresh instead of fetching it again",
  );
  assert(
    source.includes("let refreshInFlight: Promise<void> | null = null") &&
      source.includes("queuedDevicePushToken = devicePushToken"),
    "token refreshes must be single-flight and coalesce changes received in flight",
  );

  const listenerStart = source.indexOf(
    "const tokenListener = Notifications.addPushTokenListener",
  );
  const listenerEnd = source.indexOf(
    "const appStateListener = AppState.addEventListener",
    listenerStart,
  );
  const listenerBody = source.slice(listenerStart, listenerEnd);
  assert(
    listenerStart >= 0 &&
      listenerEnd > listenerStart &&
      !listenerBody.includes("getDevicePushTokenAsync"),
    "the listener must never trigger device-token acquisition directly",
  );
});
