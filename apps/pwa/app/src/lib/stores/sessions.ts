import { writable, get } from 'svelte/store';
import { apiClient } from '$api/client';
import { workspaceStore } from './workspace';

export interface Session {
	id: string;
	title: string;
	updated_at: string;
	token_count?: number | null;
	parent_session_id?: string | null;
	working_dir?: string | null;
	target_branch?: string | null;
}

interface SessionsState {
	sessions: Session[];
	directories: string[];
	isLoading: boolean;
	error: string | null;
}

const initialState: SessionsState = {
	sessions: [],
	directories: [],
	isLoading: false,
	error: null
};

export const sessionsStore = writable<SessionsState>(initialState);

// Get last directory from workspace store
export function getLastDirectory(): string | null {
	return workspaceStore.getState().directory;
}

export async function loadSessions() {
	sessionsStore.update((s) => ({ ...s, isLoading: true }));

	try {
		const data = await apiClient.getSessions();
		sessionsStore.update((s) => ({
			...s,
			sessions: data,
			isLoading: false
		}));
	} catch (err) {
		sessionsStore.update((s) => ({
			...s,
			isLoading: false,
			error: err instanceof Error ? err.message : 'Failed to load sessions'
		}));
	}
}

export async function loadDirectories() {
	try {
		const dirs = await apiClient.getDirectories();
		sessionsStore.update((s) => ({ ...s, directories: dirs }));
	} catch (err) {
		console.error('Failed to load directories:', err);
	}
}

export async function createSession(title?: string, workingDir?: string, targetBranch?: string) {
	sessionsStore.update((s) => ({ ...s, isLoading: true }));

	try {
		const data = await apiClient.createSession(title, workingDir, targetBranch);
		const state = get(sessionsStore);

		sessionsStore.update((s) => ({
			...s,
			sessions: [data, ...s.sessions],
			isLoading: false
		}));

		// Refresh directories if new one was added
		if (workingDir && !state.directories.includes(workingDir)) {
			loadDirectories();
		}

		// Update workspace store - this syncs IDE, terminal, and other tabs
		workspaceStore.setWorkspace(workingDir ?? null, data.id);

		return data;
	} catch (err) {
		sessionsStore.update((s) => ({
			...s,
			isLoading: false,
			error: err instanceof Error ? err.message : 'Failed to create session'
		}));
		return null;
	}
}

export async function selectSession(id: string) {
	const state = get(sessionsStore);
	let session = state.sessions.find((s) => s.id === id);

	// If session not in list (race condition on page load), try to fetch it
	if (!session) {
		try {
			const data = await apiClient.getSession(id);
			session = data.session;
		} catch {
			// Session doesn't exist, just set ID without directory
		}
	}

	// Update workspace store - this syncs everything
	workspaceStore.setWorkspace(session?.working_dir ?? null, id);
}

export async function deleteSession(id: string) {
	try {
		await apiClient.deleteSession(id);
		sessionsStore.update((s) => ({
			...s,
			sessions: s.sessions.filter((session) => session.id !== id)
		}));

		// Clear workspace if we deleted the current session
		const wsState = workspaceStore.getState();
		if (wsState.sessionId === id) {
			workspaceStore.clear();
		}
	} catch (err) {
		sessionsStore.update((s) => ({
			...s,
			error: err instanceof Error ? err.message : 'Failed to delete session'
		}));
	}
}
