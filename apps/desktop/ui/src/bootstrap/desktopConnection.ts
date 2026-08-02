/**
 * Desktop connection bootstrap.
 *
 * Prefer Tauri-injected server globals when present. For local Expo web
 * development, probe common localhost Mitsuro ports and auto-connect with the
 * local token so desktop never depends on mobile onboarding chrome.
 */

const CANDIDATE_PORTS = [3000, 3001, 8080, 8443];

async function probeHealth(baseUrl: string): Promise<boolean> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 1200);
    const response = await fetch(`${baseUrl.replace(/\/+$/, '')}/health`, {
      method: 'GET',
      headers: { Accept: 'application/json' },
      signal: controller.signal,
    });
    clearTimeout(timer);
    if (!response.ok) return false;
    const data = await response.json().catch(() => null);
    return Boolean(data && (data.status === 'ok' || data.features));
  } catch {
    return false;
  }
}

async function findLocalServer(): Promise<string | null> {
  for (const port of CANDIDATE_PORTS) {
    const baseUrl = `http://127.0.0.1:${port}`;
    if (typeof window !== 'undefined' && String(window.location.port || '') === String(port)) {
      continue;
    }
    if (await probeHealth(baseUrl)) return baseUrl;
  }
  return null;
}

export async function ensureDesktopServerGlobals(): Promise<void> {
  if (typeof window === 'undefined') return;

  const existingUrl = (window as any).__KRUSTY_SERVER_URL;
  const existingToken = (window as any).__KRUSTY_SERVER_TOKEN;
  if (existingUrl && existingToken) return;

  // Local desktop web only.
  const host = window.location.hostname;
  const isLocalHost = host === 'localhost' || host === '127.0.0.1' || host === '::1';
  if (!isLocalHost) return;

  // Retry briefly so a just-started local server can come up.
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const baseUrl = await findLocalServer();
    if (baseUrl) {
      (window as any).__KRUSTY_SERVER_URL = baseUrl;
      (window as any).__KRUSTY_SERVER_TOKEN = 'local';
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}
