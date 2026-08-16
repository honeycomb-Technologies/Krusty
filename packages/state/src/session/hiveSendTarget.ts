/**
 * Where a hive-mode composer send should go.
 *
 * Hive mode never creates ad-hoc sessions from the composer. When the hive
 * session store already holds a hive session (the durable companion or a
 * Worker DM), the send belongs to that loaded session. Only when nothing is
 * loaded does the caller ensure the durable companion first.
 */
export interface HiveSendTargetState {
  sessionId: string | null;
  sessionType: string | null;
}

export type HiveSendTarget =
  | { kind: "loaded-session"; sessionId: string }
  | { kind: "ensure-companion" };

export function resolveHiveSendTarget(state: HiveSendTargetState): HiveSendTarget {
  const sessionId = state.sessionId?.trim();
  if (sessionId && state.sessionType === "hive") {
    return { kind: "loaded-session", sessionId };
  }
  return { kind: "ensure-companion" };
}
