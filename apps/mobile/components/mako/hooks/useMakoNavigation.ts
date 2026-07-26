import { useCallback, useEffect, useState } from "react";
import type { MakoRunSection, MakoTopLevelView } from "../types";

export function useMakoNavigation() {
  const [topLevel, setTopLevel] = useState<MakoTopLevelView>("mako");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [runSection, setRunSection] = useState<MakoRunSection>("overview");

  const openRun = useCallback((runId: string) => {
    setSelectedRunId(runId);
    setRunSection("overview");
  }, []);

  const closeRun = useCallback(() => {
    setSelectedRunId(null);
    setTopLevel("mako");
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
