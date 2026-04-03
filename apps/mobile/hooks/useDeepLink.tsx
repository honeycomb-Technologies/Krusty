import { useEffect, useRef } from 'react';
import * as Linking from '../platform/linking';
import { useConnection } from './useConnection';

/**
 * Handles krusty:// deep links for seamless server connection.
 *
 * Supported URLs:
 *   krusty://connect?url=https://device.ts.net:8443&token=kr_remote_...
 *
 * Also handles HTTPS universal links:
 *   https://device.ts.net:8443/#krusty-remote-token=kr_remote_...
 */
export function useDeepLink() {
  const { connect } = useConnection();
  const handledRef = useRef<string | null>(null);
  const connectRef = useRef(connect);
  connectRef.current = connect;

  useEffect(() => {
    Linking.getInitialURL().then(url => {
      if (url) handleUrl(url);
    });

    const subscription = Linking.addEventListener('url', ({ url }) => {
      handleUrl(url);
    });

    return () => subscription.remove();
  }, []);

  function handleUrl(url: string) {
    // Dedupe — don't handle the same URL twice
    if (handledRef.current === url) return;
    handledRef.current = url;

    // krusty://connect?url=...&token=...
    if (url.startsWith('krusty://connect')) {
      const params = parseQueryParams(url);
      const serverUrl = params.url;
      const token = params.token;

      if (serverUrl && token) {
        connectRef.current(serverUrl, token);
      }
      return;
    }

    // https://.../#krusty-remote-token=... (hash-based launch URL)
    if (url.includes('krusty-remote-token=')) {
      const hashPart = url.split('#')[1];
      if (hashPart) {
        const hashParams = new URLSearchParams(hashPart);
        const token = hashParams.get('krusty-remote-token');
        // Extract base URL (everything before the hash)
        const serverUrl = url.split('#')[0].replace(/\/+$/, '');

        if (token && serverUrl) {
          connectRef.current(serverUrl, token);
        }
      }
      return;
    }

    // exp+krusty://connect?url=...&token=... (Expo development scheme)
    if (url.includes('connect') && url.includes('url=') && url.includes('token=')) {
      const params = parseQueryParams(url);
      const serverUrl = params.url;
      const token = params.token;

      if (serverUrl && token) {
        connectRef.current(serverUrl, token);
      }
    }
  }
}

function parseQueryParams(url: string): Record<string, string> {
  const params: Record<string, string> = {};
  const queryStart = url.indexOf('?');
  if (queryStart === -1) return params;

  const query = url.slice(queryStart + 1);
  for (const pair of query.split('&')) {
    const [key, ...rest] = pair.split('=');
    if (key) {
      params[decodeURIComponent(key)] = decodeURIComponent(rest.join('='));
    }
  }
  return params;
}
