import { useCallback, useEffect, useState } from "react";
import type { HiveRunSection, HiveTopLevelView } from "../types";

export function useHiveNavigation() {
  const [topLevel, setTopLevel] = useState<HiveTopLevelView>("hive");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [runSection, setRunSection] = useState<HiveRunSection>("overview");

  const openRun = useCallback((runId: string) => {
    setSelectedRunId(runId);
    setRunSection("overview");
  }, []);

  const closeRun = useCallback(() => {
    setSelectedRunId(null);
    setTopLevel("hive");
    setRunSection("overview");
  }, []);

  return {
    topLevel,
    setTopLevel,
    selectedRunId,
    openRun,
    closeRun,
    runSection,
    setRunSection,
  };
}
