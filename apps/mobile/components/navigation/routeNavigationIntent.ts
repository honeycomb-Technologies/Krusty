export type RouteParamValue = string | string[] | undefined;

export interface RouteNavigationParams {
  sessionId?: RouteParamValue;
  focus?: RouteParamValue;
  messageId?: RouteParamValue;
  reportId?: RouteParamValue;
}

export interface RouteNavigationIntent {
  key: string;
  params: Record<string, string>;
}

function firstRouteParam(value: RouteParamValue): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

/**
 * Route parameters are an arrival intent, not durable navigation state.
 * The stable key lets the screen consume one deep link exactly once while
 * still admitting a genuinely new notification or browser URL later.
 */
export function resolveRouteNavigationIntent(
  routeParams: RouteNavigationParams,
): RouteNavigationIntent | null {
  const sessionId = firstRouteParam(routeParams.sessionId);
  const focus = firstRouteParam(routeParams.focus);
  const messageId = firstRouteParam(routeParams.messageId);
  const reportId = firstRouteParam(routeParams.reportId);

  if (!sessionId && !focus && !messageId && !reportId) {
    return null;
  }

  return {
    key: JSON.stringify([
      sessionId ?? null,
      focus ?? null,
      messageId ?? null,
      reportId ?? null,
    ]),
    params: {
      ...(sessionId ? { sessionId } : {}),
      ...(focus ? { focus } : {}),
      ...(messageId ? { messageId } : {}),
      ...(reportId ? { reportId } : {}),
    },
  };
}
