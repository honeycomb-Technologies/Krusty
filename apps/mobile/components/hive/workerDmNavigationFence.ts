export interface WorkerDmNavigationFence {
  mount(): void;
  unmount(): void;
  beginIntent(): number;
  invalidate(): void;
  isCurrent(intent: number): boolean;
}

/**
 * Lets a Worker mutation finish while preventing its late result from
 * navigating after the owning surface or user intent has changed.
 */
export function createWorkerDmNavigationFence(): WorkerDmNavigationFence {
  let generation = 0;
  let mounted = false;

  return {
    mount() {
      mounted = true;
      generation += 1;
    },
    unmount() {
      mounted = false;
      generation += 1;
    },
    beginIntent() {
      generation += 1;
      return generation;
    },
    invalidate() {
      generation += 1;
    },
    isCurrent(intent) {
      return mounted && intent === generation;
    },
  };
}
