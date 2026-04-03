import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from 'react';
import * as SecureStore from '../platform/secure-store';
import { platformFetch } from '../platform/fetch';
import { KrustyClient } from '@krusty/api';

type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

interface ConnectionContextValue {
  client: KrustyClient | null;
  status: ConnectionStatus;
  isConnected: boolean;
  isConfigured: boolean;
  serverUrl: string | null;
  error: string | null;
  connect: (url: string, token: string) => Promise<boolean>;
  disconnect: () => void;
  reconnect: () => Promise<void>;
}

const ConnectionContext = createContext<ConnectionContextValue>({
  client: null,
  status: 'disconnected',
  isConnected: false,
  isConfigured: false,
  serverUrl: null,
  error: null,
  connect: async () => false,
  disconnect: () => {},
  reconnect: async () => {},
});

const STORAGE_KEYS = {
  serverUrl: 'krusty_server_url',
  serverToken: 'krusty_server_token',
} as const;

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [client, setClient] = useState<KrustyClient | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');
  const [serverUrl, setServerUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isConfigured, setIsConfigured] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Load saved connection on mount — check for Tauri injected URL first
  useEffect(() => {
    (async () => {
      // Tauri desktop: auto-connect via injected globals
      const tauriUrl = typeof window !== 'undefined' && (window as any).__KRUSTY_SERVER_URL;
      const tauriToken = typeof window !== 'undefined' && (window as any).__KRUSTY_SERVER_TOKEN;
      if (tauriUrl && tauriToken) {
        setIsConfigured(true);
        setServerUrl(tauriUrl);
        await doConnect(tauriUrl, tauriToken);
        setLoaded(true);
        return;
      }

      const savedUrl = await SecureStore.getItemAsync(STORAGE_KEYS.serverUrl);
      const savedToken = await SecureStore.getItemAsync(STORAGE_KEYS.serverToken);
      if (savedUrl && savedToken) {
        setIsConfigured(true);
        setServerUrl(savedUrl);
        await doConnect(savedUrl, savedToken);
      }
      setLoaded(true);
    })();
  }, []);

  const doConnect = async (url: string, token: string): Promise<boolean> => {
    setStatus('connecting');
    setError(null);

    try {
      const newClient = new KrustyClient({ baseUrl: url, token, ...(platformFetch ? { fetchImpl: platformFetch as unknown as typeof fetch } : {}) });

      // Verify connection with health check
      const healthy = await newClient.checkHealth();
      if (!healthy) {
        setStatus('error');
        setError('Server not reachable');
        return false;
      }

      // Bootstrap remote auth
      const authed = await newClient.bootstrapAuth();
      if (!authed) {
        setStatus('error');
        setError('Authentication failed — check your token');
        return false;
      }

      setClient(newClient);
      setServerUrl(url);
      setStatus('connected');
      return true;
    } catch (err) {
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Connection failed');
      return false;
    }
  };

  const connect = useCallback(async (url: string, token: string): Promise<boolean> => {
    const success = await doConnect(url, token);
    if (success) {
      await SecureStore.setItemAsync(STORAGE_KEYS.serverUrl, url);
      await SecureStore.setItemAsync(STORAGE_KEYS.serverToken, token);
      setIsConfigured(true);
    }
    return success;
  }, []);

  const disconnect = useCallback(() => {
    setClient(null);
    setStatus('disconnected');
    setServerUrl(null);
    setError(null);
    SecureStore.deleteItemAsync(STORAGE_KEYS.serverUrl);
    SecureStore.deleteItemAsync(STORAGE_KEYS.serverToken);
    setIsConfigured(false);
  }, []);

  const reconnect = useCallback(async () => {
    const savedUrl = await SecureStore.getItemAsync(STORAGE_KEYS.serverUrl);
    const savedToken = await SecureStore.getItemAsync(STORAGE_KEYS.serverToken);
    if (savedUrl && savedToken) {
      await doConnect(savedUrl, savedToken);
    }
  }, []);

  if (!loaded) return null;

  return (
    <ConnectionContext.Provider
      value={{
        client,
        status,
        isConnected: status === 'connected',
        isConfigured,
        serverUrl,
        error,
        connect,
        disconnect,
        reconnect,
      }}
    >
      {children}
    </ConnectionContext.Provider>
  );
}

export function useConnection() {
  return useContext(ConnectionContext);
}
