export async function getInitialURL(): Promise<string | null> {
  return window.location.href;
}

export function addEventListener(type: string, handler: (event: { url: string }) => void) {
  if (type !== 'url') return { remove: () => {} };

  const listener = () => handler({ url: window.location.href });
  window.addEventListener('popstate', listener);

  return { remove: () => window.removeEventListener('popstate', listener) };
}
