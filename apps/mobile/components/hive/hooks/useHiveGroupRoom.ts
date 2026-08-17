import { useCallback, useEffect, useRef, useState } from "react";
import type {
  HiveGroupDetail,
  HiveGroupEvent,
  HiveGroupMessage,
  HiveGroupTurn,
} from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export interface HiveGroupRoomState {
  detail: HiveGroupDetail | null;
  messages: HiveGroupMessage[];
  turn: HiveGroupTurn | null;
  isLoading: boolean;
  isSending: boolean;
  isStopping: boolean;
  error: string | null;
  send: (message: string) => Promise<void>;
  stop: () => Promise<void>;
  refresh: () => Promise<void>;
}

/** Coalesce SSE-driven appends so a burst becomes one state update. */
const EVENT_FLUSH_MS = 120;
const INITIAL_MESSAGE_PAGE = 100;

/**
 * Owns one group room: initial detail + recent timeline, then a room event
 * tail (message appends and turn transitions) while the room stays open.
 * The subscription belongs to this hook alone; closing the room tears it
 * down, so hidden surfaces never keep a stream alive.
 */
export function useHiveGroupRoom(
  groupId: string | null,
  enabled: boolean,
): HiveGroupRoomState {
  const { client, isConnected } = useConnection();
  const [detail, setDetail] = useState<HiveGroupDetail | null>(null);
  const [messages, setMessages] = useState<HiveGroupMessage[]>([]);
  const [turn, setTurn] = useState<HiveGroupTurn | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const latestSeqRef = useRef(0);
  const pendingRef = useRef<{
    messages: HiveGroupMessage[];
    turn: HiveGroupTurn | null;
  }>({ messages: [], turn: null });
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const appendMessages = useCallback((incoming: HiveGroupMessage[]) => {
    if (incoming.length === 0) return;
    setMessages((current) => {
      const seen = new Set(current.map((message) => message.id));
      const fresh = incoming.filter((message) => !seen.has(message.id));
      if (fresh.length === 0) return current;
      const next = [...current, ...fresh];
      next.sort((a, b) => a.seq - b.seq);
      return next;
    });
  }, []);

  const flushPending = useCallback(() => {
    flushTimerRef.current = null;
    const pending = pendingRef.current;
    pendingRef.current = { messages: [], turn: null };
    appendMessages(pending.messages);
    if (pending.turn) {
      const nextTurn = pending.turn;
      setTurn(nextTurn);
      setDetail((current) =>
        current
          ? {
              ...current,
              active_turn_id: nextTurn.status === "running" ? nextTurn.id : null,
            }
          : current,
      );
    }
  }, [appendMessages]);

  const queueEvent = useCallback(
    (event: HiveGroupEvent) => {
      if (event.type === "message") {
        latestSeqRef.current = Math.max(latestSeqRef.current, event.message.seq);
        pendingRef.current.messages.push(event.message);
      } else if (event.type === "turn") {
        pendingRef.current.turn = event.turn;
      }
      if (!flushTimerRef.current) {
        flushTimerRef.current = setTimeout(flushPending, EVENT_FLUSH_MS);
      }
    },
    [flushPending],
  );

  const refresh = useCallback(async () => {
    if (!client || !isConnected || !groupId) return;
    setError(null);
    try {
      const [nextDetail, page] = await Promise.all([
        client.getHiveGroup(groupId),
        client.listHiveGroupMessages(groupId, { limit: INITIAL_MESSAGE_PAGE }),
      ]);
      latestSeqRef.current = Math.max(latestSeqRef.current, page.latest_seq);
      setDetail(nextDetail);
      setTurn(nextDetail.active_turn ?? null);
      setMessages(page.messages);
    } catch (loadError) {
      setError(
        loadError instanceof Error ? loadError.message : "Failed to load the room",
      );
    } finally {
      setIsLoading(false);
    }
  }, [client, groupId, isConnected]);

  // Initial hydration + the event tail. The tail resumes from the durable
  // sequence cursor, so reconnects are lossless.
  useEffect(() => {
    if (!enabled || !client || !isConnected || !groupId) {
      return;
    }
    setIsLoading(true);
    setMessages([]);
    setTurn(null);
    latestSeqRef.current = 0;
    const controller = new AbortController();
    let cancelled = false;

    void (async () => {
      await refresh();
      if (cancelled) return;
      for (;;) {
        try {
          await client.observeHiveGroup(groupId, queueEvent, {
            afterSeq: latestSeqRef.current,
            signal: controller.signal,
          });
        } catch {
          // Fall through to the retry delay below.
        }
        if (cancelled || controller.signal.aborted) return;
        await new Promise((resolve) => setTimeout(resolve, 1500));
        if (cancelled || controller.signal.aborted) return;
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      pendingRef.current = { messages: [], turn: null };
    };
  }, [client, enabled, groupId, isConnected, queueEvent, refresh]);

  const send = useCallback(
    async (message: string) => {
      if (!client || !isConnected || !groupId) {
        throw new Error("Not connected to the Hive server");
      }
      setIsSending(true);
      setError(null);
      try {
        await client.sendHiveGroupMessage(
          groupId,
          { message },
          `group-send-${groupId}-${Date.now()}`,
        );
      } catch (sendError) {
        setError(
          sendError instanceof Error ? sendError.message : "Failed to send",
        );
        throw sendError;
      } finally {
        setIsSending(false);
      }
    },
    [client, groupId, isConnected],
  );

  const stop = useCallback(async () => {
    if (!client || !isConnected || !groupId) return;
    setIsStopping(true);
    setError(null);
    try {
      await client.stopHiveGroup(groupId);
    } catch (stopError) {
      setError(
        stopError instanceof Error ? stopError.message : "Failed to stop the turn",
      );
    } finally {
      setIsStopping(false);
    }
  }, [client, groupId, isConnected]);

  return {
    detail,
    messages,
    turn,
    isLoading,
    isSending,
    isStopping,
    error,
    send,
    stop,
    refresh,
  };
}
