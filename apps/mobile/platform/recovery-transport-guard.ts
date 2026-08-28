const GUARDED_RECOVERY_TRANSPORTS = new Set<PropertyKey>([
  "streamChat",
  "steerSession",
  "deleteSession",
]);

// dispatchHive creates a distinct server session and has no local queued-
// successor envelope to claim, overwrite, or scrub. Once that session exists,
// its conversation sends return through guarded streamChat/steerSession.

/** Fence chat transport behind the same authority that owns durable recovery. */
export function guardRecoveryTransport<T extends object>(
  client: T,
  ensureAuthority: () => Promise<void>,
): T {
  return new Proxy(client, {
    get(target, property) {
      const value = Reflect.get(target, property, target);
      if (typeof value !== "function") return value;
      const bound = value.bind(target) as (...args: unknown[]) => unknown;
      if (!GUARDED_RECOVERY_TRANSPORTS.has(property)) return bound;
      return async (...args: unknown[]) => {
        await ensureAuthority();
        return await bound(...args);
      };
    },
  });
}
