import {
	beginKrustyPerformanceSpan,
	configureKrustyPerformance,
	getKrustyPerformanceSnapshot,
	resetKrustyPerformance,
	trackKrustyPerformanceResource,
} from '../src/performance.ts';

declare const Deno: {
	test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
	if (!condition) throw new Error(message);
}

Deno.test('performance spans and resources stay bounded and releasable', () => {
	configureKrustyPerformance(true);
	resetKrustyPerformance();

	const finish = beginKrustyPerformanceSpan('stream.flush', 'session-1');
	const release = trackKrustyPerformanceResource('stream_connections');
	assert(
		getKrustyPerformanceSnapshot().resources.stream_connections === 1,
		'active resource should be visible while tracked',
	);

	const duration = finish();
	assert(duration !== null && duration >= 0, 'enabled span should record duration');
	assert(finish() === null, 'span completion should be idempotent');
	release();
	release();

	const snapshot = getKrustyPerformanceSnapshot();
	assert(snapshot.entries.length === 1, 'one completed span should be retained');
	assert(snapshot.entries[0]?.name === 'stream.flush', 'span name should be retained');
	assert(
		snapshot.resources.stream_connections === 0,
		'resource release should be idempotent',
	);
});

Deno.test('disabled performance instrumentation is a no-op', () => {
	configureKrustyPerformance(false);
	resetKrustyPerformance();

	const finish = beginKrustyPerformanceSpan('session.open');
	const release = trackKrustyPerformanceResource('session_requests');
	release();

	assert(finish() === null, 'disabled span should not record work');
	assert(
		getKrustyPerformanceSnapshot().entries.length === 0,
		'disabled instrumentation should keep no entries',
	);
});
