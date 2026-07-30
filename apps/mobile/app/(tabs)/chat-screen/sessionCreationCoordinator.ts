import type { SessionType } from "@krusty/api";

type IsCurrent = () => boolean;

interface PendingCreation<T> {
  generation: number;
  promise: Promise<T>;
}

export interface SessionCreationCoordinator<T> {
  hasPending(sessionType: SessionType): boolean;
  invalidate(sessionType: SessionType): void;
  run(
    sessionType: SessionType,
    task: (isCurrent: IsCurrent) => Promise<T>,
  ): Promise<T>;
}

/**
 * Owns the single durable-session creation allowed for each product mode.
 *
 * A New action and an immediate Send action therefore await the same request.
 * Invalidating a mode prevents a late response from binding over a newer
 * explicit session selection without trying to cancel the server request.
 */
export function createSessionCreationCoordinator<T>(): SessionCreationCoordinator<T> {
  const generations: Record<SessionType, number> = {
    chat: 0,
    code: 0,
    mako: 0,
  };
  const pending = new Map<SessionType, PendingCreation<T>>();

  return {
    hasPending(sessionType) {
      const existing = pending.get(sessionType);
      return existing?.generation === generations[sessionType];
    },

    invalidate(sessionType) {
      generations[sessionType] += 1;
    },

    run(sessionType, task) {
      const existing = pending.get(sessionType);
      if (existing?.generation === generations[sessionType]) {
        return existing.promise;
      }

      const generation = generations[sessionType] + 1;
      generations[sessionType] = generation;
      const isCurrent = () => generations[sessionType] === generation;
      const promise = task(isCurrent).finally(() => {
        if (pending.get(sessionType)?.generation === generation) {
          pending.delete(sessionType);
        }
      });
      pending.set(sessionType, { generation, promise });
      return promise;
    },
  };
}
