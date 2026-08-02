export type OpenGhosttyResult = {
  ok: boolean;
  message: string;
  command?: string;
};

function hasTauriBridge(): boolean {
  if (typeof window === 'undefined') return false;
  const w = window as any;
  return Boolean(
    w.__TAURI__ ||
      w.__TAURI_INTERNALS__ ||
      w.__KRUSTY_DESKTOP_HOST === true,
  );
}

async function invokeTauri<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const w = window as any;
  if (w.__TAURI__?.core?.invoke) {
    return w.__TAURI__.core.invoke(command, args) as Promise<T>;
  }
  if (w.__TAURI__?.tauri?.invoke) {
    return w.__TAURI__.tauri.invoke(command, args) as Promise<T>;
  }
  if (typeof w.__TAURI_INTERNALS__?.invoke === 'function') {
    return w.__TAURI_INTERNALS__.invoke(command, args) as Promise<T>;
  }
  throw new Error('Tauri invoke bridge unavailable');
}

export function buildGhosttyOpenCommand(directory?: string | null): string {
  const dir = directory?.trim();
  if (dir) {
    // macOS-first command shown to users; also works as documentation.
    return `open -na Ghostty --args --working-directory=${JSON.stringify(dir).slice(1, -1)}`;
  }
  return 'open -na Ghostty';
}

/**
 * Open Ghostty in the given directory.
 * - In Tauri host: uses native open_ghostty command.
 * - In local web/dev: returns a copyable command; native launch requires host bridge.
 */
export async function openGhostty(directory?: string | null): Promise<OpenGhosttyResult> {
  const dir = directory?.trim() || null;
  const command = buildGhosttyOpenCommand(dir);

  if (hasTauriBridge()) {
    try {
      const result = await invokeTauri<OpenGhosttyResult>('open_ghostty', {
        directory: dir,
      });
      return {
        ...result,
        command: result.command || command,
      };
    } catch (error) {
      return {
        ok: false,
        message: error instanceof Error ? error.message : 'Failed to open Ghostty',
        command,
      };
    }
  }

  // Best-effort: if user is on macOS Safari/Chrome with custom protocol support later,
  // we still surface the exact command for one-click copy.
  return {
    ok: false,
    message: dir
      ? `Ghostty host bridge unavailable in pure web mode. Use the command to open Ghostty in ${dir}.`
      : 'Ghostty host bridge unavailable in pure web mode. Use the command below, or run via Tauri shell for one-click open.',
    command,
  };
}

export function isDesktopHost(): boolean {
  return hasTauriBridge();
}
