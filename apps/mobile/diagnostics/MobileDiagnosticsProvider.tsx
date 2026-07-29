import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { Platform } from 'react-native';
import Constants from 'expo-constants';
import { useSegments } from 'expo-router';
import {
  MobileDiagnosticRecorder,
  buildDiagnosticUploadBatch,
  createStressDiagnosticRecorder,
  getKrustyPerformanceSnapshot,
  type DiagnosticBatch,
  type DiagnosticMode,
  type DiagnosticUploadClient,
} from '@krusty/state';

import { useConnection } from '../hooks/useConnection';
import {
  installMobileDiagnosticRecorder,
  recordMobileDiagnostic,
} from './mobileDiagnostics';
import KrustyDiagnosticsModule, {
  type NativeMetricKitPayload,
} from '../modules/krusty-diagnostics';

const INSTALLATION_KEY = 'krusty:diagnostics:installation-v1';
const PENDING_KEY = 'krusty:diagnostics:pending-v1';
const BASELINE_HEARTBEAT_MS = 1_000;
const STRESS_HEARTBEAT_MS = 250;
const PERSIST_INTERVAL_MS = 15_000;
const UPLOAD_INTERVAL_MS = 30_000;

interface DiagnosticsContextValue {
  mode: DiagnosticMode;
  runId: string | null;
  eventCount: number;
  nativePayloadCount: number;
  approximateBytes: number;
  uploadState: 'idle' | 'pending' | 'uploading' | 'uploaded' | 'failed' | 'unavailable';
  completionPending: boolean;
  startStressRun: (durationMs?: number) => void;
  stopStressRun: () => Promise<boolean>;
  flush: (completed?: boolean) => Promise<boolean>;
}

const DiagnosticsContext = createContext<DiagnosticsContextValue>({
  mode: 'baseline',
  runId: null,
  eventCount: 0,
  nativePayloadCount: 0,
  approximateBytes: 0,
  uploadState: 'idle',
  completionPending: false,
  startStressRun: () => {},
  stopStressRun: async () => false,
  flush: async () => false,
});

interface DiagnosticsGlobal {
  __KRUSTY_MOBILE_DIAGNOSTICS__?: DiagnosticsContextValue & {
    snapshot: () => ReturnType<MobileDiagnosticRecorder['snapshot']> | null;
  };
  PerformanceObserver?: new (
    callback: (list: { getEntries(): Array<{ name?: string; duration?: number }> }) => void,
  ) => {
    observe(options: Record<string, unknown>): void;
    disconnect(): void;
  };
}

interface PersistedDiagnosticsStateV2 {
  schemaVersion: 2;
  batch: DiagnosticBatch;
  completionPending: boolean;
  completedAtMs: number | null;
  mode: DiagnosticMode;
  stressEndsAtMs: number | null;
}

export function MobileDiagnosticsProvider({ children }: { children: ReactNode }) {
  const { client, isConnected } = useConnection();
  const segments = useSegments();
  const [recorder, setRecorder] = useState<MobileDiagnosticRecorder | null>(null);
  const [mode, setMode] = useState<DiagnosticMode>('baseline');
  const [summary, setSummary] = useState({ eventCount: 0, approximateBytes: 0 });
  const [nativePayloadCount, setNativePayloadCount] = useState(0);
  const [uploadState, setUploadState] = useState<DiagnosticsContextValue['uploadState']>('idle');
  const appStateRef = useRef<AppStateStatus>(AppState.currentState);
  const persistedRunIdRef = useRef<string | null>(null);
  const persistedRevisionRef = useRef(-1);
  const persistPromiseRef = useRef<Promise<boolean> | null>(null);
  const uploadingRef = useRef(false);
  const pendingCompletionRef = useRef(false);
  const seenPerformanceEntriesRef = useRef(new WeakSet<object>());
  const previousResourcesRef = useRef<Record<string, number>>({});
  const lastRouteRef = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const bootstrap = await readDiagnosticsBootstrap(
        () => cancelled,
        () => setUploadState('failed'),
      );
      if (cancelled || !bootstrap) return;
      const { installationId, storedPending } = bootstrap;
      setUploadState((current) => current === 'failed' ? 'idle' : current);
      let next: MobileDiagnosticRecorder;
      if (storedPending) {
        try {
          const parsed = JSON.parse(storedPending) as DiagnosticBatch | PersistedDiagnosticsStateV2;
          if (isPersistedDiagnosticsStateV2(parsed, installationId)) {
            next = new MobileDiagnosticRecorder({
              installationId,
              runId: parsed.batch.runId,
              startedAtMs: parsed.batch.runStartedAtMs,
            });
            if (parsed.mode === 'stress' && parsed.stressEndsAtMs !== null) {
              next.resumeActiveStress(parsed.stressEndsAtMs);
            }
            next.restore(parsed.batch);
            if (parsed.completionPending) {
              next.resumeStressCompletion(parsed.completedAtMs ?? parsed.batch.createdAtMs);
              pendingCompletionRef.current = true;
              setUploadState('pending');
            }
          } else {
            next = new MobileDiagnosticRecorder({
              installationId,
              runId: createPseudonymousId('run'),
            });
            next.restore(parsed as DiagnosticBatch);
          }
        } catch {
          // Corrupt or legacy pending diagnostics are intentionally discarded.
          next = new MobileDiagnosticRecorder({
            installationId,
            runId: createPseudonymousId('run'),
          });
        }
      } else {
        next = new MobileDiagnosticRecorder({
          installationId,
          runId: createPseudonymousId('run'),
        });
      }
      next.record('app', { name: 'diagnostics.ready' });
      const restoredMode = next.getMode();
      setMode(restoredMode);
      if (next.consumeStressCompletion()) {
        pendingCompletionRef.current = true;
        setUploadState('pending');
      }
      installMobileDiagnosticRecorder(next);
      setRecorder(next);
    })();
    return () => {
      cancelled = true;
      installMobileDiagnosticRecorder(null);
    };
  }, []);

  const persist = useCallback((targetRecorder = recorder): Promise<boolean> => {
    const previous = persistPromiseRef.current ?? Promise.resolve(true);
    const operation = previous.then(async () => {
      if (!targetRecorder) return true;
      if (
        persistedRunIdRef.current === targetRecorder.runId
        && persistedRevisionRef.current === targetRecorder.getRevision()
      ) return true;
      const revision = targetRecorder.getRevision();
      const mode = targetRecorder.getMode();
      const completionPending = pendingCompletionRef.current
        || targetRecorder.isStressCompletionPending();
      const retainFullRun = completionPending || mode === 'stress';
      const batch = retainFullRun
        ? targetRecorder.createCompletionPersistenceBatch()
        : targetRecorder.createPersistenceBatch();
      try {
        if (!batch) {
          await AsyncStorage.removeItem(PENDING_KEY);
        } else {
          const state: PersistedDiagnosticsStateV2 = {
            schemaVersion: 2,
            batch,
            completionPending,
            completedAtMs: targetRecorder.getStressCompletedAtMs(),
            mode,
            stressEndsAtMs: targetRecorder.getStressEndsAtMs(),
          };
          await AsyncStorage.setItem(PENDING_KEY, JSON.stringify(state));
        }
        persistedRunIdRef.current = targetRecorder.runId;
        persistedRevisionRef.current = revision;
        return true;
      } catch {
        return false;
      }
    });
    persistPromiseRef.current = operation;
    void operation.finally(() => {
      if (persistPromiseRef.current === operation) persistPromiseRef.current = null;
    });
    return operation;
  }, [recorder]);

  const flush = useCallback(async (completed = false): Promise<boolean> => {
    if (completed) pendingCompletionRef.current = true;
    if (!recorder || !client || !isConnected || uploadingRef.current) {
      if (completed) await persist();
      if (completed && !uploadingRef.current) setUploadState('pending');
      return false;
    }
    const upload = (client as unknown as Partial<DiagnosticUploadClient>)
      .uploadMobileDiagnostics;
    if (typeof upload !== 'function') {
      setUploadState('unavailable');
      return false;
    }
    uploadingRef.current = true;
    setUploadState('uploading');
    try {
      let nativePayloads: NativeMetricKitPayload[] = KrustyDiagnosticsModule
        ? await KrustyDiagnosticsModule.listMetricKitPayloads().catch(() => [])
        : [];
      setNativePayloadCount(nativePayloads.length);
      if (!recorder.createBatch() && nativePayloads.length > 0 && !recorder.isStressCompletionPending()) {
        recorder.record('diagnostic', { name: 'native.payloads', count: nativePayloads.length });
      }
      let batch = recorder.createBatch() ?? createNativeOnlyBatch(recorder, nativePayloads);
      if (!batch) {
        setUploadState('uploaded');
        return true;
      }
      // Calling through the configured KrustyClient preserves its authenticated
      // transport. There is deliberately no raw URL/token fallback here.
      const completesRun = completed || pendingCompletionRef.current;
      while (batch) {
        const isFinalBatch = batch.events.length === recorder.snapshot().eventCount;
        const completesThisBatch = completesRun && isFinalBatch;
        await upload.call(client, buildDiagnosticUploadBatch(batch, {
          appVersion: Constants.nativeAppVersion ?? Constants.expoConfig?.version ?? 'unknown',
          buildNumber: Constants.nativeBuildVersion ?? 'unknown',
          platform: Platform.OS === 'ios' || Platform.OS === 'android'
            ? Platform.OS
            : 'web',
          osVersion: String(Platform.Version ?? 'unknown'),
          deviceClass: Platform.OS === 'web' ? 'web' : 'mobile',
          captureLevel: completesRun ? 'stress' : recorder.getMode(),
          completed: completesThisBatch,
          endedAtMs: completesThisBatch ? recorder.getStressCompletedAtMs() : null,
        }, nativePayloads));
        recorder.acknowledge(batch.events.map((event) => event.id));
        if (KrustyDiagnosticsModule && nativePayloads.length > 0) {
          try {
            await KrustyDiagnosticsModule
              .acknowledgeMetricKitPayloads(nativePayloads.map((payload) => payload.id));
            setNativePayloadCount(0);
            nativePayloads = [];
          } catch {
            await persist();
            setUploadState('failed');
            return false;
          }
        }
        await persist();
        if (!completesRun) break;
        batch = recorder.createBatch();
      }
      if (completesRun) {
        try {
          await AsyncStorage.removeItem(PENDING_KEY);
        } catch {
          setUploadState('failed');
          return false;
        }
        pendingCompletionRef.current = false;
        const next = new MobileDiagnosticRecorder({
          installationId: recorder.installationId,
          runId: createPseudonymousId('run'),
        });
        next.record('app', { name: 'diagnostics.ready' });
        persistedRunIdRef.current = null;
        persistedRevisionRef.current = -1;
        installMobileDiagnosticRecorder(next);
        setRecorder(next);
        setMode('baseline');
        setSummary({ eventCount: 1, approximateBytes: next.snapshot().approximateBytes });
      }
      setUploadState('uploaded');
      return true;
    } catch {
      await persist();
      setUploadState('failed');
      return false;
    } finally {
      uploadingRef.current = false;
    }
  }, [client, isConnected, persist, recorder]);

  const startStressRun = useCallback((durationMs?: number) => {
    if (!recorder || pendingCompletionRef.current || uploadingRef.current) return;
    const next = createStressDiagnosticRecorder({
      installationId: recorder.installationId,
      runId: createPseudonymousId('run'),
      durationMs,
    });
    pendingCompletionRef.current = false;
    persistedRunIdRef.current = null;
    persistedRevisionRef.current = -1;
    installMobileDiagnosticRecorder(next);
    setRecorder(next);
    setUploadState('idle');
    setMode(next.getMode());
    const snapshot = next.snapshot();
    setSummary({
      eventCount: snapshot.eventCount,
      approximateBytes: snapshot.approximateBytes,
    });
    // Persist the run envelope immediately. Periodic uploads may acknowledge
    // every queued event while capture remains active; the empty envelope must
    // still retain the run identity, mode, and deadline across force-quit.
    void persist(next);
  }, [persist, recorder]);

  const stopStressRun = useCallback(async () => {
    recorder?.stopStressRun();
    pendingCompletionRef.current = true;
    if (!isConnected) setUploadState('pending');
    setMode('baseline');
    await persist();
    return flush(true);
  }, [flush, isConnected, persist, recorder]);

  const context = useMemo<DiagnosticsContextValue>(() => ({
    mode,
    runId: recorder?.runId ?? null,
    eventCount: summary.eventCount,
    nativePayloadCount,
    approximateBytes: summary.approximateBytes,
    uploadState,
    completionPending: pendingCompletionRef.current
      || Boolean(recorder?.isStressCompletionPending()),
    startStressRun,
    stopStressRun,
    flush,
  }), [
    flush,
    mode,
    nativePayloadCount,
    recorder?.runId,
    recorder,
    startStressRun,
    stopStressRun,
    summary.approximateBytes,
    summary.eventCount,
    uploadState,
  ]);

  useEffect(() => {
    if (!recorder) return;
    if (!KrustyDiagnosticsModule) return;
    let cancelled = false;
    void KrustyDiagnosticsModule.listMetricKitPayloads()
      .then((payloads) => {
        if (!cancelled) setNativePayloadCount(payloads.length);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [recorder]);

  useEffect(() => {
    if (!recorder) return;
    const route = classifyRoute(segments as string[]);
    if (lastRouteRef.current === route) return;
    lastRouteRef.current = route;
    recorder.record('navigation', { name: route });
  }, [recorder, segments]);

  useEffect(() => {
    if (!recorder) return;
    recorder.record('app_state', { state: AppState.currentState });
    const subscription = AppState.addEventListener('change', (nextState) => {
      appStateRef.current = nextState;
      recorder.record('app_state', { state: nextState });
      if (nextState !== 'active') void persist();
    });
    return () => subscription.remove();
  }, [persist, recorder]);

  useEffect(() => {
    if (!recorder) return;
    const intervalMs = mode === 'stress' ? STRESS_HEARTBEAT_MS : BASELINE_HEARTBEAT_MS;
    const thresholdMs = mode === 'stress' ? 40 : 250;
    let expectedAt = Date.now() + intervalMs;
    const timer = setInterval(() => {
      const now = Date.now();
      const drift = Math.max(0, now - expectedAt);
      expectedAt = now + intervalMs;
      if (appStateRef.current === 'active' && drift >= thresholdMs) {
        recorder.record('heartbeat', { name: 'js.drift', durationMs: drift });
      }
      const nextMode = recorder.getMode();
      if (nextMode !== mode) {
        setMode(nextMode);
        if (recorder.consumeStressCompletion()) {
          pendingCompletionRef.current = true;
          void persist().then(() => flush(true));
        }
      }
      const snapshot = recorder.snapshot();
      setSummary((current) =>
        current.eventCount === snapshot.eventCount
          && current.approximateBytes === snapshot.approximateBytes
          ? current
          : {
              eventCount: snapshot.eventCount,
              approximateBytes: snapshot.approximateBytes,
            });
    }, intervalMs);
    return () => clearInterval(timer);
  }, [flush, mode, persist, recorder]);

  useEffect(() => {
    if (!recorder) return;
    const root = globalThis as typeof globalThis & DiagnosticsGlobal;
    const Observer = root.PerformanceObserver;
    if (!Observer) return;
    const observers: Array<{ disconnect(): void }> = [];
    const observe = (type: 'longtask' | 'event') => {
      try {
        const observer = new Observer((list) => {
          for (const entry of list.getEntries()) {
            const duration = Number(entry.duration ?? 0);
            const threshold = recorder.getMode() === 'stress'
              ? type === 'longtask' ? 16 : 8
              : type === 'longtask' ? 50 : 100;
            if (duration < threshold) continue;
            recorder.record(type === 'longtask' ? 'longtask' : 'event_timing', {
              name: type === 'event' ? safeEventName(entry.name) : 'js.longtask',
              durationMs: duration,
            });
          }
        });
        (observer.observe as (options: Record<string, unknown>) => void)({
          type,
          buffered: false,
          durationThreshold: type === 'event' ? 8 : undefined,
        });
        observers.push(observer);
      } catch {
        // Performance entry type is unavailable on this RN/runtime build.
      }
    };
    observe('longtask');
    observe('event');
    return () => observers.forEach((observer) => observer.disconnect());
  }, [recorder]);

  useEffect(() => {
    if (!recorder) return;
    const timer = setInterval(() => {
      const snapshot = getKrustyPerformanceSnapshot();
      for (const entry of snapshot.entries) {
        if (seenPerformanceEntriesRef.current.has(entry)) continue;
        seenPerformanceEntriesRef.current.add(entry);
        if (entry.name === 'mode.switch') {
          recorder.record('mode', { name: safeModeTransition(entry.detail), durationMs: entry.durationMs });
        } else {
          recorder.record('performance', { name: entry.name, durationMs: entry.durationMs });
        }
      }
      for (const [name, count] of Object.entries(snapshot.resources)) {
        if (previousResourcesRef.current[name] === count) continue;
        previousResourcesRef.current[name] = count ?? 0;
        recorder.record('resource', { name, count });
      }
    }, mode === 'stress' ? 1_000 : 5_000);
    return () => clearInterval(timer);
  }, [mode, recorder]);

  useEffect(() => {
    if (!recorder) return;
    const persistTimer = setInterval(() => void persist(), PERSIST_INTERVAL_MS);
    const uploadTimer = setInterval(() => {
      if (AppState.currentState === 'active') void flush();
    }, UPLOAD_INTERVAL_MS);
    return () => {
      clearInterval(persistTimer);
      clearInterval(uploadTimer);
      void persist();
    };
  }, [flush, persist, recorder]);

  useEffect(() => {
    const root = globalThis as typeof globalThis & DiagnosticsGlobal;
    root.__KRUSTY_MOBILE_DIAGNOSTICS__ = {
      ...context,
      snapshot: () => recorder?.snapshot() ?? null,
    };
    return () => {
      delete root.__KRUSTY_MOBILE_DIAGNOSTICS__;
    };
  }, [context, recorder]);

  return (
    <DiagnosticsContext.Provider value={context}>
      {children}
    </DiagnosticsContext.Provider>
  );
}

export function useMobileDiagnostics(): DiagnosticsContextValue {
  return useContext(DiagnosticsContext);
}

export function useMobileDiagnosticMode(mode: 'chat' | 'code' | 'mako'): void {
  useEffect(() => {
    recordMobileDiagnostic('mode', { state: mode });
  }, [mode]);
}

function createPseudonymousId(prefix: 'install' | 'run'): string {
  const random = globalThis.crypto?.randomUUID?.()
    ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 14)}`;
  return `${prefix}-${random}`;
}

function validIdentifier(value: string | null): value is string {
  return Boolean(value && /^(install|run)-[a-zA-Z0-9-]{8,64}$/.test(value));
}

async function readDiagnosticsBootstrap(
  isCancelled: () => boolean,
  onRetry: () => void,
): Promise<{ installationId: string; storedPending: string | null } | null> {
  let candidateInstallationId: string | null = null;
  while (!isCancelled()) {
    const [installationResult, pendingResult] = await Promise.allSettled([
      AsyncStorage.getItem(INSTALLATION_KEY),
      AsyncStorage.getItem(PENDING_KEY),
    ]);
    if (installationResult.status === 'rejected' || pendingResult.status === 'rejected') {
      onRetry();
      await diagnosticsRetryDelay();
      continue;
    }

    const storedInstallationId = installationResult.value;
    const installationId: string = validIdentifier(storedInstallationId)
      ? storedInstallationId
      : candidateInstallationId ?? createPseudonymousId('install');
    if (!validIdentifier(storedInstallationId)) {
      candidateInstallationId = installationId;
      try {
        await AsyncStorage.setItem(INSTALLATION_KEY, installationId);
      } catch {
        onRetry();
        await diagnosticsRetryDelay();
        continue;
      }
    }
    return { installationId, storedPending: pendingResult.value };
  }
  return null;
}

function diagnosticsRetryDelay(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 1_000));
}

function isPersistedDiagnosticsStateV2(
  value: DiagnosticBatch | PersistedDiagnosticsStateV2,
  installationId: string,
): value is PersistedDiagnosticsStateV2 {
  return Boolean(
    value
    && 'schemaVersion' in value
    && value.schemaVersion === 2
    && 'batch' in value
    && value.batch?.installationId === installationId
    && validIdentifier(value.batch.runId),
  );
}

function createNativeOnlyBatch(
  recorder: MobileDiagnosticRecorder,
  payloads: readonly NativeMetricKitPayload[],
): DiagnosticBatch | null {
  if (payloads.length === 0) return null;
  return {
    schemaVersion: 1,
    installationId: recorder.installationId,
    runId: recorder.runId,
    runStartedAtMs: recorder.startedAtMs,
    createdAtMs: Date.now(),
    events: [],
  };
}

function classifyRoute(segments: string[]): string {
  const allowed = new Set(['(tabs)', 'index', 'sessions', 'settings', 'onboarding']);
  return segments.map((segment) => allowed.has(segment) ? segment : 'dynamic').join('>') || 'root';
}

function safeEventName(name: string | undefined): string {
  return ['click', 'pointerdown', 'pointerup', 'keydown', 'keyup', 'touchstart', 'touchend']
    .includes(name ?? '') ? name! : 'interaction';
}

function safeModeTransition(detail: string | undefined): string {
  return /^(chat|code|mako)->(chat|code|mako)$/.test(detail ?? '')
    ? detail!
    : 'mode.change';
}
