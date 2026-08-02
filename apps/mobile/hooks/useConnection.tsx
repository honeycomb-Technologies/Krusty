import { createContext, useContext, useState, useEffect, useCallback, type ReactNode } from 'react';
import * as SecureStore from '../platform/secure-store';
import { platformFetch } from '../platform/fetch';
import { MitsuroClient } from '@mitsuro/api';
import { recordRequestDiagnostic } from '../diagnostics/mobileDiagnostics';
import {
  deleteConnectionCredentials,
  readConnectionCredentials,
  writeConnectionCredentials,
} from '../platform/identity-storage';
import { connectionFromInjectedGlobals } from '../platform/identity-compatibility';

type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

interface ConnectionContextValue {
  client: MitsuroClient | null;
  status: ConnectionStatus;
  isConnected: boolean;
  isConfigured: boolean;
  hasLoadedConnection: boolean;
  serverUrl: string | null;
  serverToken: string | null;
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
  hasLoadedConnection: false,
  serverUrl: null,
  serverToken: null,
  error: null,
  connect: async () => false,
  disconnect: () => {},
  reconnect: async () => {},
});

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [client, setClient] = useState<MitsuroClient | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('disconnected');
  const [serverUrl, setServerUrl] = useState<string | null>(null);
  const [serverToken, setServerToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isConfigured, setIsConfigured] = useState(false);
  const [loaded, setLoaded] = useState(false);

  // Load saved connection on mount — check for Tauri injected URL first
  useEffect(() => {
    (async () => {
      // Tauri desktop: auto-connect via injected globals
      const tauriConnection = typeof window === 'undefined'
        ? null
        : connectionFromInjectedGlobals(window as unknown as Record<string, unknown>);
      if (tauriConnection) {
        setIsConfigured(true);
        setServerUrl(tauriConnection.serverUrl);
        await doConnect(tauriConnection.serverUrl, tauriConnection.token);
        setLoaded(true);
        return;
      }

      try {
        const saved = await readConnectionCredentials(SecureStore);
        if (saved) {
          setIsConfigured(true);
          setServerUrl(saved.serverUrl);
          await doConnect(saved.serverUrl, saved.token);
        }
      } catch (loadError) {
        setStatus('error');
        setError(loadError instanceof Error ? loadError.message : 'Stored connection is invalid');
      }
      setLoaded(true);
    })();
  }, []);

  const doConnect = async (url: string, token: string): Promise<boolean> => {
    setStatus('connecting');
    setError(null);

    try {
      const newClient = new MitsuroClient({
        baseUrl: url,
        token,
        ...(platformFetch
          ? { fetchImpl: platformFetch as unknown as typeof fetch }
          : {}),
        requestObserver: ({ name, outcome, durationMs, code }) => {
          recordRequestDiagnostic(name, outcome, durationMs, code);
        },
      });

      // Verify connection with health check
      const healthy = await newClient.checkHealth();
      if (!healthy) {
        setClient(null);
        setServerToken(null);
        setStatus('error');
        setError('Server not reachable');
        return false;
      }

      // Bootstrap remote auth
      const authed = await newClient.bootstrapAuth();
      if (!authed) {
        setClient(null);
        setServerToken(null);
        setStatus('error');
        setError('Authentication failed — check your token');
        return false;
      }

      setClient(newClient);
      setServerUrl(url);
      setServerToken(token);
      setStatus('connected');
      return true;
    } catch (err) {
      setClient(null);
      setServerToken(null);
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Connection failed');
      return false;
    }
  };

  const connect = useCallback(async (url: string, token: string): Promise<boolean> => {
    const success = await doConnect(url, token);
    if (success) {
      await writeConnectionCredentials(SecureStore, { serverUrl: url, token });
      setIsConfigured(true);
    }
    return success;
  }, []);

  const disconnect = useCallback(() => {
    setClient(null);
    setStatus('disconnected');
    setServerUrl(null);
    setServerToken(null);
    setError(null);
    void deleteConnectionCredentials(SecureStore);
    setIsConfigured(false);
  }, []);

  const reconnect = useCallback(async () => {
    const saved = await readConnectionCredentials(SecureStore);
    if (saved) {
      await doConnect(saved.serverUrl, saved.token);
    }
  }, []);

  return (
    <ConnectionContext.Provider
      value={{
        client,
        status,
        isConnected: status === 'connected',
        isConfigured,
        hasLoadedConnection: loaded,
        serverUrl,
        serverToken,
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
