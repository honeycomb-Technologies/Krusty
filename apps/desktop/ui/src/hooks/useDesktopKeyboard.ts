import { useEffect } from 'react';
import type { DesktopPlane } from '../shell/types';

interface Options {
  onPlane: (plane: DesktopPlane) => void;
  onToggleContext: () => void;
  onToggleUtility: () => void;
  onOpenSettings: () => void;
  onNewSession: () => void;
}

export function useDesktopKeyboard({
  onPlane,
  onToggleContext,
  onToggleUtility,
  onOpenSettings,
  onNewSession,
}: Options) {
  useEffect(() => {
    if (typeof window === 'undefined') return;

    const onKeyDown = (event: KeyboardEvent) => {
      const meta = event.metaKey || event.ctrlKey;
      if (!meta) return;

      if (event.key === '1') {
        event.preventDefault();
        onPlane('chat');
      } else if (event.key === '2') {
        event.preventDefault();
        onPlane('code');
      } else if (event.key === '3') {
        event.preventDefault();
        onPlane('hive');
      } else if (event.key.toLowerCase() === 'b') {
        event.preventDefault();
        onToggleContext();
      } else if (event.key.toLowerCase() === 'j') {
        event.preventDefault();
        onToggleUtility();
      } else if (event.key === ',') {
        event.preventDefault();
        onOpenSettings();
      } else if (event.key.toLowerCase() === 'n') {
        event.preventDefault();
        onNewSession();
      }
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onNewSession, onOpenSettings, onPlane, onToggleContext, onToggleUtility]);
}
