import {
	beginMitsuroPerformanceSpan,
	configureMitsuroPerformance,
	getMitsuroPerformanceSnapshot,
	recordMitsuroPerformanceMetric,
	resetMitsuroPerformance,
	trackMitsuroPerformanceResource,
} from '../src/performance.ts';

declare const Deno: {
	test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
	if (!condition) throw new Error(message);
}

Deno.test('performance spans and resources stay bounded and releasable', () => {
	configureMitsuroPerformance(true);
	resetMitsuroPerformance();

	const finish = beginMitsuroPerformanceSpan('stream.flush', 'session-1');
	const release = trackMitsuroPerformanceResource('stream_connections');
	assert(
		getMitsuroPerformanceSnapshot().resources.stream_connections === 1,
		'active resource should be visible while tracked',
	);

	const duration = finish();
	recordMitsuroPerformanceMetric('session.snapshot_max_slice', {
		durationMs: 3.25,
	});
	recordMitsuroPerformanceMetric('session.snapshot_yields', { count: 4 });
	assert(duration !== null && duration >= 0, 'enabled span should record duration');
	assert(finish() === null, 'span completion should be idempotent');
	release();
	release();

	const snapshot = getMitsuroPerformanceSnapshot();
	assert(snapshot.entries.length === 3, 'span and numeric metrics should be retained');
	assert(snapshot.entries[0]?.name === 'stream.flush', 'span name should be retained');
	assert(
		snapshot.entries[1]?.durationMs === 3.25,
		'maximum synchronous slice timing should remain numeric',
	);
	assert(
		snapshot.entries[2]?.count === 4,
		'cooperative yield count should remain numeric',
	);
	assert(
		snapshot.resources.stream_connections === 0,
		'resource release should be idempotent',
	);
});

Deno.test('disabled performance instrumentation is a no-op', () => {
	configureMitsuroPerformance(false);
	resetMitsuroPerformance();

	const finish = beginMitsuroPerformanceSpan('session.open');
	const release = trackMitsuroPerformanceResource('session_requests');
	release();

	assert(finish() === null, 'disabled span should not record work');
	assert(
		getMitsuroPerformanceSnapshot().entries.length === 0,
		'disabled instrumentation should keep no entries',
	);
});
