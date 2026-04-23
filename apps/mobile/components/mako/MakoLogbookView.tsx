import { MakoReportsView } from "./MakoReportsView";

interface MakoLogbookViewProps {
  workspaceDirectory?: string | null;
}

export function MakoLogbookView({ workspaceDirectory }: MakoLogbookViewProps) {
  return <MakoReportsView workspaceDirectory={workspaceDirectory} />;
}
