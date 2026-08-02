import { HiveReportsView } from "./HiveReportsView";

interface HiveLogbookViewProps {
  workspaceDirectory?: string | null;
}

export function HiveLogbookView({ workspaceDirectory }: HiveLogbookViewProps) {
  return <HiveReportsView workspaceDirectory={workspaceDirectory} />;
}
