type IsCurrentRefresh = () => boolean;

interface PendingRefresh {
  generation: number;
  promise: Promise<void>;
}

type RefreshTask = (isCurrent: IsCurrentRefresh) => Promise<void>;

export interface WorkerRosterRefreshCoordinator {
  invalidate(): void;
  run(task: RefreshTask): Promise<void>;
  runAfterCommit(task: RefreshTask): Promise<void>;
}

/**
 * Coalesces ordinary roster reads while letting a committed mutation supersede
 * every read that began before it. Superseded requests may finish, but their
 * `isCurrent` fence can no longer adopt stale Worker state.
 */
export function createWorkerRosterRefreshCoordinator(): WorkerRosterRefreshCoordinator {
  let generation = 0;
  let pending: PendingRefresh | null = null;

  const start = (task: RefreshTask): Promise<void> => {
    const requestGeneration = ++generation;
    const promise = task(() => generation === requestGeneration);
    pending = { generation: requestGeneration, promise };
    void promise
      .finally(() => {
        if (pending?.generation === requestGeneration) pending = null;
      })
      .catch(() => undefined);
    return promise;
  };

  return {
    invalidate() {
      generation += 1;
    },

    run(task) {
      if (pending?.generation === generation) return pending.promise;
      return start(task);
    },

    runAfterCommit(task) {
      // Invalidate an older single-flight before starting the mandatory
      // post-commit read. It may remain in flight, but it cannot be adopted.
      generation += 1;
      return start(task);
    },
  };
}
