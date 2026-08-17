import { useCallback, useEffect, useRef, useState } from "react";
import type { HiveWorkerDelivery } from "@mitsuro/api";
import { useConnection } from "../../../hooks/useConnection";

export interface HiveWakeItem extends HiveWorkerDelivery {
  workerSlug: string;
  workerName: string;
}

export function useHiveWorkerDeliveries({
  workerId = null,
  enabled,
  limit = 8,
}: {
  workerId?: string | null;
  enabled: boolean;
  limit?: number;
}): {
  items: HiveWakeItem[];
  isLoading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const { client, isConnected } = useConnection();
  const [items, setItems] = useState<HiveWakeItem[]>([]);
  const [isLoading, setIsLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);

  const refresh = useCallback(async () => {
    if (!enabled || !client || !isConnected) {
      setItems([]);
      setIsLoading(false);
      return;
    }

    const generation = ++generationRef.current;
    setError(null);
    setIsLoading(true);
    try {
      const workers = workerId
        ? [{ id: workerId, slug: "", display_name: "" }]
        : (await client.listHiveWorkers()).workers;

      const pages = await Promise.all(
        workers.map(async (worker) => {
          const response = await client.listHiveWorkerDeliveries(worker.id, {
            limit,
          });
          return response.deliveries.map((delivery) => ({
            ...delivery,
            workerSlug: worker.slug,
            workerName: worker.display_name,
          }));
        }),
      );
      if (generation !== generationRef.current) {
        return;
      }
      const merged = pages.flat().sort((left, right) => {
        return right.created_at.localeCompare(left.created_at);
      });
      setItems(merged.slice(0, workerId ? limit : 12));
    } catch (loadError) {
      if (generation !== generationRef.current) {
        return;
      }
      setError(
        loadError instanceof Error
          ? loadError.message
          : "Failed to load Worker wakes",
      );
    } finally {
      if (generation === generationRef.current) {
        setIsLoading(false);
      }
    }
  }, [client, enabled, isConnected, limit, workerId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { items, isLoading, error, refresh };
}

export function wakeReasonLabel(item: HiveWorkerDelivery): string {
  if (item.kind === "worker_message") {
    return item.group_id ? "Group peer message" : "Direct peer message";
  }
  return item.kind.replace(/_/g, " ");
}

export function wakeStatusLabel(status: HiveWorkerDelivery["status"]): string {
  switch (status) {
    case "pending":
      return "queued";
    case "delivering":
      return "waking";
    case "delivered":
      return "delivered";
    case "acked":
      return "done";
    case "dead_letter":
      return "failed";
    default:
      return status;
  }
}
