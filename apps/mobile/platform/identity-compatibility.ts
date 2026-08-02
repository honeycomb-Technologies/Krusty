export const CANONICAL_DEEP_LINK_SCHEME = "mitsuro";
export const CANONICAL_REMOTE_TOKEN_HASH_KEY = "mitsuro-remote-token";
const LEGACY_DEEP_LINK_SCHEME = "krusty";
const LEGACY_REMOTE_TOKEN_HASH_KEY = "krusty-remote-token";

export const HIVE_NOTIFICATION_CATEGORY = "HIVE_SESSION";
export const HIVE_NOTIFICATION_ACTION = "OPEN_HIVE";
export const LEGACY_HIVE_NOTIFICATION_CATEGORY = "MAKO_SESSION";
export const LEGACY_HIVE_NOTIFICATION_ACTION = "OPEN_MAKO";

export type ConnectionLaunch = {
  serverUrl: string;
  token: string;
};

type DesktopConnectionGlobals = Partial<Record<
  | "__MITSURO_SERVER_URL"
  | "__MITSURO_SERVER_TOKEN"
  | "__KRUSTY_SERVER_URL"
  | "__KRUSTY_SERVER_TOKEN",
  unknown
>>;

/** Read one complete injected-global generation; never mix URL and token generations. */
export function connectionFromInjectedGlobals(
  globals: DesktopConnectionGlobals,
): ConnectionLaunch | null {
  const canonical = connectionLaunch(
    stringValue(globals.__MITSURO_SERVER_URL),
    stringValue(globals.__MITSURO_SERVER_TOKEN),
  );
  if (canonical) return canonical;
  return connectionLaunch(
    stringValue(globals.__KRUSTY_SERVER_URL),
    stringValue(globals.__KRUSTY_SERVER_TOKEN),
  );
}

export function parseConnectionLaunchUrl(url: string): ConnectionLaunch | null {
  if (
    url.startsWith(`${CANONICAL_DEEP_LINK_SCHEME}://connect`) ||
    url.startsWith(`${LEGACY_DEEP_LINK_SCHEME}://connect`)
  ) {
    const params = parseQueryParams(url);
    return connectionLaunch(params.url, params.token);
  }

  const hashPart = url.split("#", 2)[1];
  if (hashPart) {
    const hashParams = new URLSearchParams(hashPart);
    const token =
      hashParams.get(CANONICAL_REMOTE_TOKEN_HASH_KEY) ??
      hashParams.get(LEGACY_REMOTE_TOKEN_HASH_KEY);
    const serverUrl = url.split("#", 1)[0]?.replace(/\/+$/, "");
    const launch = connectionLaunch(serverUrl, token);
    if (launch) return launch;
  }

  // Expo development schemes include an environment-specific prefix.
  if (url.includes("connect") && url.includes("url=") && url.includes("token=")) {
    const params = parseQueryParams(url);
    return connectionLaunch(params.url, params.token);
  }
  return null;
}

export function canonicalNotificationAction(actionIdentifier: string): string {
  return actionIdentifier === LEGACY_HIVE_NOTIFICATION_ACTION
    ? HIVE_NOTIFICATION_ACTION
    : actionIdentifier;
}

export function canonicalNotificationData<T extends Record<string, unknown>>(
  data: T,
): T {
  let normalized: Record<string, unknown> | null = null;
  if (data.type === "mako_update") {
    normalized = { ...data, type: "hive_update" };
  }
  if (data.focus === "mako") {
    normalized = { ...(normalized ?? data), focus: "hive" };
  }
  return (normalized ?? data) as T;
}

function connectionLaunch(
  serverUrl: string | null | undefined,
  token: string | null | undefined,
): ConnectionLaunch | null {
  return serverUrl && token ? { serverUrl, token } : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value ? value : null;
}

function parseQueryParams(url: string): Record<string, string> {
  const queryStart = url.indexOf("?");
  if (queryStart === -1) return {};
  const params = new URLSearchParams(url.slice(queryStart + 1));
  return Object.fromEntries(params.entries());
}
