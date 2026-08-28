import type {
  HiveWorkerSessionBindingResponse,
  SessionType,
} from "@mitsuro/api";

export type HiveSessionBindingKind = HiveWorkerSessionBindingResponse["kind"];

export type GenericSessionDeleteDisposition =
  | "allowed"
  | "worker_dm"
  | "unresolved";

export interface GenericSessionDeleteTarget {
  sessionId: string;
  sessionType: SessionType | null | undefined;
  workerDmSessionIds?: ReadonlySet<string>;
  bindingKind?: HiveSessionBindingKind | null;
}

/** Generic delete is allowed only for a known non-Hive or proven primary Hive session. */
export function genericSessionDeleteDisposition(
  target: GenericSessionDeleteTarget,
): GenericSessionDeleteDisposition {
  if (target.sessionType === "chat" || target.sessionType === "code") {
    return "allowed";
  }
  if (target.sessionType !== "hive") {
    return "unresolved";
  }
  if (
    target.workerDmSessionIds?.has(target.sessionId) ||
    target.bindingKind === "worker_dm"
  ) {
    return "worker_dm";
  }
  return target.bindingKind === "primary_hive" ? "allowed" : "unresolved";
}

interface ResolvedGenericSessionDeleteTarget
  extends GenericSessionDeleteTarget {
  resolveHiveBinding?: (
    sessionId: string,
  ) => Promise<HiveWorkerSessionBindingResponse>;
}

export async function resolveGenericSessionDeleteDisposition(
  target: ResolvedGenericSessionDeleteTarget,
): Promise<GenericSessionDeleteDisposition> {
  const known = genericSessionDeleteDisposition(target);
  if (known !== "unresolved") return known;
  if (target.sessionType !== "hive" || !target.resolveHiveBinding) {
    return "unresolved";
  }

  try {
    const binding = await target.resolveHiveBinding(target.sessionId);
    if (binding.session_id !== target.sessionId) return "unresolved";
    return genericSessionDeleteDisposition({
      ...target,
      bindingKind: binding.kind,
    });
  } catch {
    return "unresolved";
  }
}

/**
 * Runs the destructive boundary only after exact classification. Worker DMs
 * and lookup failures never reach local clearing or the generic DELETE call.
 */
export async function runGenericSessionDeleteIfAllowed(
  target: ResolvedGenericSessionDeleteTarget,
  onAllowed: () => void | Promise<void>,
): Promise<GenericSessionDeleteDisposition> {
  const disposition = await resolveGenericSessionDeleteDisposition(target);
  if (disposition === "allowed") await onAllowed();
  return disposition;
}
