export type MitsuroPerformancePhase =
	| 'app.launch'
	| 'new_chat.shell'
	| 'new_chat.session_bind'
	| 'session.open'
	| 'stream.connect'
	| 'stream.first_event'
	| 'stream.flush'
	| 'stream.finish'
	| 'session.fetch_decode'
	| 'session.snapshot_transform'
	| 'session.snapshot_publish'
	| 'session.cache_compact'
	| 'transcript.derive'
	| 'transcript.visual_plan'
	| 'transcript.first_paint'
	| 'mode.switch'
	| 'toolbox.open'
	| 'diagnostics.persist'
	| 'diagnostics.upload'
	| 'live_activity.update';

export type MitsuroPerformanceMetric =
	| 'session.snapshot_max_slice'
	| 'session.snapshot_yields'
	| 'transcript.visible_messages'
	| 'transcript.visible_render_parts'
	| 'transcript.visible_tools'
	| 'transcript.visible_markdown_characters';

export type MitsuroPerformanceResource =
	| 'stream_connections'
	| 'state_polling'
	| 'presence_heartbeats'
	| 'session_requests'
	| 'toolbox_requests'
	| 'live_activity_updates';

export interface MitsuroPerformanceEntry {
	name: MitsuroPerformancePhase | MitsuroPerformanceMetric;
	durationMs: number;
	count?: number;
	startedAtMs: number;
	detail?: string;
}

export interface MitsuroPerformanceSnapshot {
	enabled: boolean;
	entries: MitsuroPerformanceEntry[];
	resources: Partial<Record<MitsuroPerformanceResource, number>>;
}

interface PerformanceLike {
	now(): number;
	mark?(name: string): void;
	measure?(name: string, startMark?: string, endMark?: string): void;
	clearMarks?(name?: string): void;
}

interface MitsuroPerformanceGlobal {
	performance?: PerformanceLike;
	__MITSURO_NATIVE_PERFORMANCE__?: {
		begin(spanId: number, name: MitsuroPerformancePhase): void;
		end(spanId: number, name: MitsuroPerformancePhase): void;
	};
	__MITSURO_PERFORMANCE__?: {
		snapshot: () => MitsuroPerformanceSnapshot;
		reset: () => void;
	};
}

const MAX_ENTRIES = 256;
const entries: MitsuroPerformanceEntry[] = [];
const resources: Partial<Record<MitsuroPerformanceResource, number>> = {};
let enabled = false;
let nextSpanId = 0;

function performanceGlobal(): MitsuroPerformanceGlobal {
	return globalThis as typeof globalThis & MitsuroPerformanceGlobal;
}

function clockNow(): number {
	return performanceGlobal().performance?.now() ?? Date.now();
}

function snapshot(): MitsuroPerformanceSnapshot {
	return {
		enabled,
		entries: entries.slice(),
		resources: { ...resources },
	};
}

export function resetMitsuroPerformance(): void {
	entries.length = 0;
	for (const key of Object.keys(resources) as MitsuroPerformanceResource[]) {
		delete resources[key];
	}
}

export function configureMitsuroPerformance(nextEnabled: boolean): void {
	enabled = nextEnabled;
	const root = performanceGlobal();
	root.__MITSURO_PERFORMANCE__ = {
		snapshot,
		reset: resetMitsuroPerformance,
	};
}

export function getMitsuroPerformanceSnapshot(): MitsuroPerformanceSnapshot {
	return snapshot();
}

export function beginMitsuroPerformanceSpan(
	name: MitsuroPerformancePhase,
	detail?: string,
): () => number | null {
	if (!enabled) {
		return () => null;
	}

	const root = performanceGlobal();
	const startedAtMs = clockNow();
	const spanId = nextSpanId++;
	const startMark = `mitsuro.${name}.${spanId}.start`;
	const endMark = `mitsuro.${name}.${spanId}.end`;
	root.performance?.mark?.(startMark);
	root.__MITSURO_NATIVE_PERFORMANCE__?.begin(spanId, name);
	let ended = false;

	return () => {
		if (ended) return null;
		ended = true;
		const durationMs = Math.max(0, clockNow() - startedAtMs);
		root.performance?.mark?.(endMark);
		root.performance?.measure?.(`mitsuro.${name}`, startMark, endMark);
		root.performance?.clearMarks?.(startMark);
		root.performance?.clearMarks?.(endMark);
		root.__MITSURO_NATIVE_PERFORMANCE__?.end(spanId, name);
		entries.push({ name, durationMs, startedAtMs, detail });
		if (entries.length > MAX_ENTRIES) {
			entries.splice(0, entries.length - MAX_ENTRIES);
		}
		return durationMs;
	};
}

export function recordMitsuroPerformanceMetric(
	name: MitsuroPerformanceMetric,
	values: { durationMs?: number; count?: number },
): void {
	if (!enabled) return;
	const durationMs = Number.isFinite(values.durationMs)
		? Math.max(0, values.durationMs ?? 0)
		: 0;
	const count = Number.isFinite(values.count)
		? Math.max(0, Math.floor(values.count ?? 0))
		: undefined;
	entries.push({
		name,
		durationMs,
		count,
		startedAtMs: clockNow(),
	});
	if (entries.length > MAX_ENTRIES) {
		entries.splice(0, entries.length - MAX_ENTRIES);
	}
}

export function setMitsuroPerformanceResource(
	name: MitsuroPerformanceResource,
	value: number,
): void {
	if (!enabled) return;
	resources[name] = Math.max(0, Math.floor(value));
}

export function trackMitsuroPerformanceResource(
	name: MitsuroPerformanceResource,
): () => void {
	if (!enabled) return () => {};
	resources[name] = (resources[name] ?? 0) + 1;
	let released = false;
	return () => {
		if (released) return;
		released = true;
		resources[name] = Math.max(0, (resources[name] ?? 0) - 1);
	};
}
