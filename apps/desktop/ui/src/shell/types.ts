import type { SessionType } from '@mitsuro/api';

export type DesktopPlane = SessionType;
export type DesktopUtilityPane =
  | 'none'
  | 'terminal'
  | 'changes'
  | 'browser'
  | 'library'
  | 'connections'
  | 'schedule'
  | 'runs'
  | 'memory';

export type DesktopSettingsOpen = boolean;
