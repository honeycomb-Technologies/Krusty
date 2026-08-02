import MitsuroDiagnosticsModule from '../modules/mitsuro-diagnostics';

const ARRAY_FROM_SAMPLE_EVERY = 1_024;
const NETWORK_STACK_SAMPLE_EVERY = 256;
const REPORT_INTERVAL_MS = 2_000;
const MAX_REPORTED_CALLSITES = 8;
const MAX_REPORTED_NETWORK_CALLS = 4;

interface JsHotPathProbeGlobal {
  __MITSURO_JS_HOT_PATH_PROBE_INSTALLED__?: boolean;
  __rctDeviceEventEmitter?: {
    emit(eventType: string, ...args: unknown[]): void;
    listenerCount?(eventType: string): number;
  };
}

interface CallsiteCount {
  callsite: string;
  count: number;
}

interface NativeEventCount {
  eventType: string;
  count: number;
  maxListeners: number;
}

interface NetworkCallCount {
  target: string;
  count: number;
  callsite: string;
}

/**
 * Installs a deliberately narrow diagnostic probe for local Release profiling.
 *
 * This is compile-time opt-in because wrapping a JavaScript builtin would be
 * inappropriate in an ordinary production build. The probe samples only one
 * in every 1,024 Array.from calls and emits bounded aggregate callsite counts.
 */
export function installJsHotPathProbe(): void {
  const enabled =
    process.env.EXPO_PUBLIC_MITSURO_JS_HOTPATH_PROBE === '1' ||
    process.env.EXPO_PUBLIC_KRUSTY_JS_HOTPATH_PROBE === '1';
  if (!enabled) return;

  const root = globalThis as typeof globalThis & JsHotPathProbeGlobal;
  if (root.__MITSURO_JS_HOT_PATH_PROBE_INSTALLED__) return;
  root.__MITSURO_JS_HOT_PATH_PROBE_INSTALLED__ = true;

  const originalArrayFrom = Array.from;
  const callsites = new Map<string, number>();
  const nativeEvents = new Map<string, NativeEventCount>();
  const networkCalls = new Map<string, NetworkCallCount>();
  const iteratorSources = new WeakMap<object, string>();
  let activeNativeEventType: string | null = null;
  let callCount = 0;
  let networkCallCount = 0;
  let previousCallCount = 0;
  let sampling = false;

  tagCollectionIterators(Map.prototype, 'Map', iteratorSources);
  tagCollectionIterators(Set.prototype, 'Set', iteratorSources);
  const deviceEventEmitter = root.__rctDeviceEventEmitter;
  if (deviceEventEmitter) {
    const originalEmit = deviceEventEmitter.emit;
    deviceEventEmitter.emit = function deviceEventProbe(
      this: typeof deviceEventEmitter,
      eventType: string,
      ...args: unknown[]
    ): void {
      const previousEventType = activeNativeEventType;
      activeNativeEventType = eventType;
      const current = nativeEvents.get(eventType);
      const listenerCount = this.listenerCount?.(eventType) ?? 0;
      nativeEvents.set(eventType, {
        eventType,
        count: (current?.count ?? 0) + 1,
        maxListeners: Math.max(current?.maxListeners ?? 0, listenerCount),
      });
      try {
        Reflect.apply(originalEmit, this, [eventType, ...args]);
      } finally {
        activeNativeEventType = previousEventType;
      }
    };
  }
  const originalFetch = globalThis.fetch;
  globalThis.fetch = function fetchProbe(
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> {
    recordNetworkCall(
      `${init?.method?.toUpperCase() ?? 'GET'} ${sanitizeNetworkTarget(input)}`,
      networkCalls,
      ++networkCallCount % NETWORK_STACK_SAMPLE_EVERY === 0,
    );
    return Reflect.apply(originalFetch, this, [input, init]) as Promise<Response>;
  };
  const originalXhrOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function networkOpenProbe(
    method: string,
    url: string | URL,
    async?: boolean,
    username?: string | null,
    password?: string | null,
  ): void {
    recordNetworkCall(
      `XHR ${method.toUpperCase()} ${sanitizeNetworkTarget(url)}`,
      networkCalls,
      ++networkCallCount % NETWORK_STACK_SAMPLE_EVERY === 0,
    );
    Reflect.apply(originalXhrOpen, this, [method, url, async, username, password]);
  };
  Error.stackTraceLimit = Math.max(Error.stackTraceLimit, 12);

  const arrayFromProbe = function (
    this: ArrayConstructor,
    ...args: unknown[]
  ): unknown[] {
    callCount += 1;
    const result = Reflect.apply(originalArrayFrom, this, args) as unknown[];
    if (!sampling && callCount % ARRAY_FROM_SAMPLE_EVERY === 0) {
      sampling = true;
      try {
        const callsite = selectCallsite(new Error().stack);
        const input = args[0];
        const source = typeof input === 'object' && input !== null
          ? iteratorSources.get(input) ?? Object.prototype.toString.call(input)
          : typeof input;
        const eventType = activeNativeEventType ?? 'none';
        const key = `${source};length=${result.length};event=${eventType};callsite=${callsite}`;
        callsites.set(key, (callsites.get(key) ?? 0) + 1);
      } finally {
        sampling = false;
      }
    }
    return result;
  } as typeof Array.from;

  Object.defineProperty(Array, 'from', {
    configurable: true,
    enumerable: false,
    value: arrayFromProbe,
    writable: true,
  });

  setInterval(() => {
    const callsSinceLastReport = callCount - previousCallCount;
    previousCallCount = callCount;
    if (callsSinceLastReport === 0) return;

    const ranked: CallsiteCount[] = [];
    callsites.forEach((count, callsite) => {
      ranked.push({ callsite, count });
    });
    ranked.sort((left, right) => right.count - left.count);
    const rankedNativeEvents = Array.from(nativeEvents.values())
      .sort((left, right) => right.count - left.count)
      .slice(0, MAX_REPORTED_CALLSITES);
    const rankedNetworkCalls = Array.from(networkCalls.values())
      .sort((left, right) => right.count - left.count)
      .slice(0, MAX_REPORTED_NETWORK_CALLS);
    const payload = JSON.stringify({
      arrayFromCalls: callsSinceLastReport,
      sampleEvery: ARRAY_FROM_SAMPLE_EVERY,
      callsites: ranked.slice(0, MAX_REPORTED_CALLSITES),
      nativeEvents: rankedNativeEvents,
      networkCalls: rankedNetworkCalls,
    });
    MitsuroDiagnosticsModule?.recordJsHotPathProbe(payload);
    callsites.clear();
    nativeEvents.clear();
    networkCalls.clear();
  }, REPORT_INTERVAL_MS);
}

function recordNetworkCall(
  target: string,
  networkCalls: Map<string, NetworkCallCount>,
  captureStack: boolean,
): void {
  const previous = networkCalls.get(target);
  networkCalls.set(target, {
    target,
    count: (previous?.count ?? 0) + 1,
    callsite: captureStack
      ? selectCallsite(new Error().stack)
      : previous?.callsite ?? 'not-sampled',
  });
}

function sanitizeNetworkTarget(input: unknown): string {
  const raw = typeof input === 'string'
    ? input
    : input instanceof URL
      ? input.toString()
      : String((input as { url?: unknown } | null)?.url ?? input);
  return raw
    .replace(/[?#].*$/, '')
    .replace(/[0-9a-f]{8}-[0-9a-f-]{27,}/gi, ':id')
    .replace(/\/[0-9]{6,}(?=\/|$)/g, '/:id')
    .slice(0, 180);
}

function tagCollectionIterators(
  prototype: Map<unknown, unknown> | Set<unknown>,
  collectionName: 'Map' | 'Set',
  iteratorSources: WeakMap<object, string>,
): void {
  for (const methodName of ['entries', 'keys', 'values'] as const) {
    const original = prototype[methodName] as () => IterableIterator<unknown>;
    Object.defineProperty(prototype, methodName, {
      configurable: true,
      enumerable: false,
      value: function taggedIterator(this: Map<unknown, unknown> | Set<unknown>) {
        const iterator = Reflect.apply(original, this, []) as object;
        iteratorSources.set(iterator, `${collectionName}.${methodName}`);
        return iterator;
      },
      writable: true,
    });
  }
}

function selectCallsite(stack: string | undefined): string {
  if (!stack) return 'unknown';
  const lines = stack.split('\n');
  for (const line of lines) {
    const trimmed = line.trim();
    if (
      !trimmed
      || trimmed === 'Error'
      || trimmed.includes('arrayFromProbe')
      || trimmed.includes('fetchProbe')
      || trimmed.includes('installJsHotPathProbe')
      || trimmed.includes('networkOpenProbe')
      || trimmed.includes('recordNetworkCall')
    ) continue;
    return trimmed.slice(0, 240);
  }
  return 'unknown';
}
