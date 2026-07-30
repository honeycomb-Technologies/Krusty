import { sanitizeDiagnosticFields } from './redaction';
import type {
  DiagnosticBatch,
  DiagnosticEvent,
  DiagnosticEventType,
  DiagnosticFields,
  DiagnosticMode,
  DiagnosticSnapshot,
} from './types';

const BASELINE_MAX_EVENTS = 256;
const STRESS_MAX_EVENTS = 2_048;
const BASELINE_MAX_BYTES = 96 * 1024;
const STRESS_MAX_BYTES = 384 * 1024;
const MAX_BATCH_EVENTS = 128;
const MAX_BATCH_BYTES = 96 * 1024;
const MAX_RESTORE_AGE_MS = 72 * 60 * 60 * 1000;
const EVENT_TYPES = new Set([
  'app', 'mode', 'navigation', 'app_state', 'heartbeat', 'longtask',
  'event_timing', 'performance', 'resource', 'request', 'webview',
  'live_activity', 'diagnostic',
]);

export interface DiagnosticRecorderOptions {
  installationId: string;
  runId: string;
  startedAtMs?: number;
  now?: () => number;
}

export interface StressDiagnosticRecorderOptions extends DiagnosticRecorderOptions {
  durationMs?: number;
}

/** Start an explicitly bounded capture with a fresh identity and empty event ring. */
export function createStressDiagnosticRecorder(
  options: StressDiagnosticRecorderOptions,
): MobileDiagnosticRecorder {
  const recorder = new MobileDiagnosticRecorder(options);
  recorder.startStressRun(options.durationMs);
  return recorder;
}

export class MobileDiagnosticRecorder {
  readonly installationId: string;
  readonly runId: string;
  readonly startedAtMs: number;
  private readonly now: () => number;
  private events: DiagnosticEvent[] = [];
  private bytes = 0;
  private sequence = 0;
  private mode: DiagnosticMode = 'baseline';
  private stressEndsAtMs: number | null = null;
  private stressCompletionPending = false;
  private stressCompletionSignaled = false;
  private stressCompletedAtMs: number | null = null;
  private revision = 0;
  private readonly restoredSourceIds = new Set<string>();

  constructor(options: DiagnosticRecorderOptions) {
    this.installationId = options.installationId;
    this.runId = options.runId;
    this.now = options.now ?? Date.now;
    this.startedAtMs = options.startedAtMs ?? this.now();
  }

  record(type: DiagnosticEventType, fields: DiagnosticFields = {}): DiagnosticEvent {
    this.expireStressIfNeeded();
    if (this.stressCompletionPending) {
      return this.buildEvent(type, fields, this.mode, false);
    }
    return this.buildEvent(type, fields, this.mode, true);
  }

  private buildEvent(
    type: DiagnosticEventType,
    fields: DiagnosticFields,
    mode: DiagnosticMode,
    store: boolean,
  ): DiagnosticEvent {
    const sequence = this.sequence++;
    const event: DiagnosticEvent = {
      id: `${this.runId}:${sequence}`,
      sequence,
      type,
      atMs: this.now(),
      mode,
      fields: sanitizeDiagnosticFields(fields),
    };
    if (!store) return event;
    this.events.push(event);
    this.bytes += eventBytes(event);
    this.revision += 1;
    this.trim();
    return event;
  }

  startStressRun(durationMs = 10 * 60 * 1000): void {
    if (this.mode === 'stress' || this.stressCompletionPending) return;
    this.mode = 'stress';
    this.stressCompletionPending = false;
    this.stressCompletionSignaled = false;
    this.stressCompletedAtMs = null;
    this.stressEndsAtMs = this.now() + Math.min(
      30 * 60 * 1000,
      Math.max(10_000, durationMs),
    );
    this.record('diagnostic', { name: 'stress.start' });
  }

  stopStressRun(): void {
    if (this.mode === 'stress') {
      this.finishStress('stress.stop');
    }
  }

  getMode(): DiagnosticMode {
    this.expireStressIfNeeded();
    return this.mode;
  }

  getStressEndsAtMs(): number | null {
    return this.stressEndsAtMs;
  }

  resumeActiveStress(stressEndsAtMs: number): void {
    if (!Number.isFinite(stressEndsAtMs) || this.stressCompletionPending) return;
    this.mode = 'stress';
    this.stressEndsAtMs = stressEndsAtMs;
    this.stressCompletionSignaled = false;
    this.stressCompletedAtMs = null;
  }

  /** True once for an explicitly stopped or automatically expired stress capture. */
  consumeStressCompletion(): boolean {
    this.expireStressIfNeeded();
    if (!this.stressCompletionPending || this.stressCompletionSignaled) return false;
    this.stressCompletionSignaled = true;
    return true;
  }

  isStressCompletionPending(): boolean {
    return this.stressCompletionPending;
  }

  getStressCompletedAtMs(): number | null {
    return this.stressCompletedAtMs;
  }

  resumeStressCompletion(completedAtMs: number): void {
    this.mode = 'baseline';
    this.stressEndsAtMs = null;
    this.stressCompletionPending = true;
    this.stressCompletionSignaled = false;
    this.stressCompletedAtMs = completedAtMs;
  }

  getRevision(): number {
    return this.revision;
  }

  snapshot(): DiagnosticSnapshot {
    this.expireStressIfNeeded();
    return {
      mode: this.mode,
      installationId: this.installationId,
      runId: this.runId,
      runStartedAtMs: this.startedAtMs,
      eventCount: this.events.length,
      approximateBytes: this.bytes,
      events: this.events.slice(),
    };
  }

  createBatch(): DiagnosticBatch | null {
    if (this.events.length === 0) return null;
    const selected: DiagnosticEvent[] = [];
    let selectedBytes = 0;
    for (const event of this.events) {
      const size = eventBytes(event);
      if (
        selected.length >= MAX_BATCH_EVENTS ||
        (selected.length > 0 && selectedBytes + size > MAX_BATCH_BYTES)
      ) {
        break;
      }
      selected.push(event);
      selectedBytes += size;
    }
    return {
      schemaVersion: 1,
      installationId: this.installationId,
      runId: this.runId,
      runStartedAtMs: this.startedAtMs,
      createdAtMs: this.now(),
      events: selected,
    };
  }

  /** Latest bounded slice for crash recovery; uploads continue oldest-first. */
  createPersistenceBatch(): DiagnosticBatch | null {
    if (this.events.length === 0) return null;
    const selected: DiagnosticEvent[] = [];
    let selectedBytes = 0;
    for (let index = this.events.length - 1; index >= 0; index -= 1) {
      const event = this.events[index]!;
      const size = eventBytes(event);
      if (
        selected.length >= MAX_BATCH_EVENTS ||
        (selected.length > 0 && selectedBytes + size > MAX_BATCH_BYTES)
      ) {
        break;
      }
      selected.unshift(event);
      selectedBytes += size;
    }
    return {
      schemaVersion: 1,
      installationId: this.installationId,
      runId: this.runId,
      runStartedAtMs: this.startedAtMs,
      createdAtMs: this.now(),
      events: selected,
    };
  }

  /** Full bounded recorder state for restart-safe completion draining. */
  createCompletionPersistenceBatch(): DiagnosticBatch {
    return {
      schemaVersion: 1,
      installationId: this.installationId,
      runId: this.runId,
      runStartedAtMs: this.startedAtMs,
      createdAtMs: this.now(),
      events: this.events.slice(),
    };
  }

  acknowledge(eventIds: readonly string[]): void {
    if (eventIds.length === 0) return;
    const acknowledged = new Set(eventIds);
    const before = this.events.length;
    this.events = this.events.filter((event) => !acknowledged.has(event.id));
    if (this.events.length !== before) {
      this.recomputeBytes();
      this.revision += 1;
    }
  }

  restore(batch: DiagnosticBatch, restoredAtMs = this.now()): number {
    if (
      batch.schemaVersion !== 1 ||
      batch.installationId !== this.installationId ||
      restoredAtMs - batch.createdAtMs > MAX_RESTORE_AGE_MS ||
      batch.createdAtMs > restoredAtMs + 60_000
    ) {
      return 0;
    }
    let restored = 0;
    const known = new Set(this.events.map((event) => event.id));
    for (const event of batch.events) {
      if (
        known.has(event.id) ||
        this.restoredSourceIds.has(event.id) ||
        !isValidPersistedEvent(event)
      ) continue;
      const preservesRun = batch.runId === this.runId;
      const sequence = preservesRun ? event.sequence : this.sequence;
      const sanitized: DiagnosticEvent = {
        id: preservesRun ? event.id : `recovered:${this.runId}:${sequence}`,
        sequence,
        type: event.type,
        atMs: Math.min(restoredAtMs, Math.max(0, event.atMs)),
        mode: event.mode === 'stress' ? 'stress' : 'baseline',
        fields: sanitizeDiagnosticFields(event.fields),
      };
      this.sequence = Math.max(this.sequence, sequence + 1);
      this.events.push(sanitized);
      this.restoredSourceIds.add(event.id);
      this.bytes += eventBytes(sanitized);
      restored += 1;
    }
    if (restored > 0) {
      this.revision += 1;
      this.trim();
      if (batch.runId !== this.runId) {
        this.record('diagnostic', { name: 'pending.recovered', count: restored });
      }
    }
    return restored;
  }

  private expireStressIfNeeded(): void {
    if (
      this.mode === 'stress' &&
      this.stressEndsAtMs !== null &&
      this.now() >= this.stressEndsAtMs
    ) {
      this.finishStress('stress.expired');
    }
  }

  private finishStress(name: 'stress.stop' | 'stress.expired'): void {
    if (this.mode !== 'stress' || this.stressCompletionPending) return;
    this.stressEndsAtMs = null;
    this.buildEvent('diagnostic', { name }, 'stress', true);
    this.mode = 'baseline';
    this.stressCompletionPending = true;
    this.stressCompletionSignaled = false;
    this.stressCompletedAtMs = this.now();
  }

  private trim(): void {
    const retainStressBounds = this.mode === 'stress' || this.stressCompletionPending;
    const maxEvents = retainStressBounds ? STRESS_MAX_EVENTS : BASELINE_MAX_EVENTS;
    const maxBytes = retainStressBounds ? STRESS_MAX_BYTES : BASELINE_MAX_BYTES;
    while (this.events.length > maxEvents || this.bytes > maxBytes) {
      const removed = this.events.shift();
      if (!removed) break;
      this.bytes = Math.max(0, this.bytes - eventBytes(removed));
    }
  }

  private recomputeBytes(): void {
    this.bytes = this.events.reduce((total, event) => total + eventBytes(event), 0);
  }
}

function eventBytes(event: DiagnosticEvent): number {
  return JSON.stringify(event).length * 2;
}

function isValidPersistedEvent(event: DiagnosticEvent): boolean {
  return Boolean(
    event &&
    typeof event.id === 'string' &&
    Number.isSafeInteger(event.sequence) &&
    EVENT_TYPES.has(event.type) &&
    Number.isFinite(event.atMs) &&
    event.fields &&
    typeof event.fields === 'object',
  );
}
