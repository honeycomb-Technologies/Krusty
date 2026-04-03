export const ImpactFeedbackStyle = {
  Light: 'light' as const,
  Medium: 'medium' as const,
  Heavy: 'heavy' as const,
};

export const NotificationFeedbackType = {
  Success: 'success' as const,
  Error: 'error' as const,
  Warning: 'warning' as const,
};

export async function impactAsync(_style?: string): Promise<void> {}
export async function notificationAsync(_type?: string): Promise<void> {}
