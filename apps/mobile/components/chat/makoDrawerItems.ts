import type { MakoTopLevelView } from "../mako/types";

export interface MakoDrawerItem {
  id: MakoTopLevelView;
  label: string;
  detail: string;
}

export const MAKO_DRAWER_ITEMS: MakoDrawerItem[] = [
  {
    id: "mako",
    label: "Mako",
    detail: "Main thread",
  },
  {
    id: "attention",
    label: "Attention",
    detail: "Approvals and blockers",
  },
  {
    id: "schedule",
    label: "Schedule",
    detail: "Agenda and calendar",
  },
  {
    id: "logbook",
    label: "Logbook",
    detail: "Reports and memory",
  },
  {
    id: "runs",
    label: "Runs",
    detail: "Active and sleeping",
  },
];
