import { useCallback, useEffect, useState } from "react";
import type { MakoRunSection, MakoTopLevelView } from "../types";

export function useMakoNavigation(activeRunId?: string | null) {
  const [topLevel, setTopLevel] = useState<MakoTopLevelView>("current");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [runSection, setRunSection] = useState<MakoRunSection>("overview");

  useEffect(() => {
    if (activeRunId) {
      setSelectedRunId(activeRunId);
    }
  }, [activeRunId]);

  const openRun = useCallback((runId: string) => {
    setSelectedRunId(runId);
    setRunSection("overview");
  }, []);

  const closeRun = useCallback(() => {
    setSelectedRunId(null);
    setTopLevel("current");
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
