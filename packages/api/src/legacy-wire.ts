/** Transition-only route and value adapters for pre-Mitsuro/Hive servers. */
const LEGACY_HIVE_ROUTE_PREFIX = '/mako';
const CANONICAL_HIVE_ROUTE_PREFIX = '/hive';

export function legacyHiveApiPath(path: string): string | null {
	if (
		path !== CANONICAL_HIVE_ROUTE_PREFIX &&
		!path.startsWith(`${CANONICAL_HIVE_ROUTE_PREFIX}/`)
	) {
		return null;
	}
	return `${LEGACY_HIVE_ROUTE_PREFIX}${path.slice(CANONICAL_HIVE_ROUTE_PREFIX.length)}`;
}

export function isLegacyRouteFallbackStatus(status: number): boolean {
	return status === 404 || status === 405;
}

/**
 * Encode the one renamed enum that can cross a non-Hive-prefixed route.
 *
 * Callers must only use this after the route probe has positively identified a
 * pre-Hive server. Keeping the old wire literal here prevents compatibility
 * vocabulary from spreading back into the canonical client.
 */
export function encodeLegacyRequestIdentity<T extends Record<string, unknown>>(
	value: T,
): T {
	if (value.session_type !== 'hive') return value;
	return { ...value, session_type: 'mako' } as T;
}

function normalizeSessionRecord(value: unknown): unknown {
	if (!value || typeof value !== 'object' || Array.isArray(value)) return value;
	const session = value as Record<string, unknown>;
	return session.session_type === 'mako'
		? { ...session, session_type: 'hive' }
		: value;
}

function normalizeSessionList(value: unknown): unknown {
	if (!Array.isArray(value)) return value;
	let changed = false;
	const sessions = value.map((session) => {
		const normalized = normalizeSessionRecord(session);
		changed ||= normalized !== session;
		return normalized;
	});
	return changed ? sessions : value;
}

function normalizeNestedSession(value: unknown): unknown {
	if (!value || typeof value !== 'object' || Array.isArray(value)) return value;
	const response = value as Record<string, unknown>;
	const session = normalizeSessionRecord(response.session);
	return session === response.session ? value : { ...response, session };
}

/**
 * Convert old session enums only at response fields owned by the session API.
 *
 * Messages, tool arguments, reports, and extension payloads are intentionally
 * opaque. A user payload that happens to contain `session_type: "mako"` must
 * survive byte-for-byte instead of being treated as an identity contract.
 */
export function normalizeLegacyResponseIdentity(
	path: string,
	value: unknown,
): unknown {
	const endpoint = path.split('?', 1)[0] ?? path;
	if (endpoint === '/sessions') {
		return Array.isArray(value)
			? normalizeSessionList(value)
			: normalizeSessionRecord(value);
	}
	if (/^\/sessions\/[^/]+$/.test(endpoint)) {
		const response = value as Record<string, unknown> | null;
		return response?.session === undefined
			? normalizeSessionRecord(value)
			: normalizeNestedSession(value);
	}
	if (/^\/sessions\/[^/]+\/pinch$/.test(endpoint)) {
		return normalizeNestedSession(value);
	}
	if (
		endpoint === '/hive/main' ||
		/^\/hive\/sessions\/[^/]+\/status$/.test(endpoint)
	) {
		return normalizeSessionRecord(value);
	}
	return value;
}
