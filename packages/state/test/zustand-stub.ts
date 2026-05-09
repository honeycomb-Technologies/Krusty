type StateCreator<T> = (
  set: (partial: Partial<T> | ((state: T) => Partial<T>)) => void,
  get: () => T,
  api: StoreApi<T>,
) => T;

type StoreApi<T> = {
  getState: () => T;
  setState: (partial: Partial<T> | ((state: T) => Partial<T>)) => void;
  subscribe: (listener: (state: T) => void) => () => void;
};

export function create<T>(initializer: StateCreator<T>) {
  let state: T;
  const listeners = new Set<(state: T) => void>();

  const api: StoreApi<T> = {
    getState: () => state,
    setState: (partial) => {
      const nextPartial = typeof partial === "function"
        ? (partial as (state: T) => Partial<T>)(state)
        : partial;
      state = { ...state, ...nextPartial };
      for (const listener of listeners) listener(state);
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };

  const set = api.setState;
  const get = api.getState;
  state = initializer(set, get, api);

  const useStore =
    ((selector?: (state: T) => unknown) =>
      selector ? selector(state) : state) as
        & typeof api
        & ((selector?: (state: T) => unknown) => unknown);
  Object.assign(useStore, api);
  return useStore;
}
