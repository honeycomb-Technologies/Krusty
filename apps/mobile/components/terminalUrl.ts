/**
 * Build the terminal WebSocket URL for the current connection.
 *
 * Remote auth currently passes the token as a query parameter because browser
 * WebSocket constructors cannot set arbitrary Authorization headers (and the
 * embedded terminal path has the same limitation). The server only accepts query
 * tokens on WebSocket upgrade requests.
 *
 * Tradeoffs / leakage risk: query tokens can appear in proxy logs, browser
 * history, and crash reports. Keep remote tokens high-entropy and short-lived
 * where possible; prefer bearer headers for non-WebSocket HTTP APIs.
 */
export function buildTerminalWebSocketUrl(
  serverUrl: string,
  token?: string | null,
): string {
  const base = serverUrl.replace(/^http/i, "ws").replace(/\/+$/, "");
  const trimmedToken = token?.trim();
  if (!trimmedToken) {
    return `${base}/ws/terminal`;
  }

  return `${base}/ws/terminal?token=${encodeURIComponent(trimmedToken)}`;
}
