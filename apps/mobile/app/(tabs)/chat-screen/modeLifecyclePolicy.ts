import type { SessionType } from "@mitsuro/api";

export interface ModeLifecyclePolicy {
  keepPresence: boolean;
  keepPolling: boolean;
}

/**
 * Hidden running modes keep their transport/recovery path but do not advertise
 * an active viewer. Hidden idle modes own neither heartbeat nor polling work.
 */
export function resolveModeLifecyclePolicy(
  activeMode: SessionType,
  mode: SessionType,
  isStreaming: boolean,
): ModeLifecyclePolicy {
  return {
    keepPresence: mode === activeMode,
    keepPolling: isStreaming,
  };
}
