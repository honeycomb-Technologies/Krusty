import type { MakoTopLevelView } from "../mako/types";

export interface MakoDrawerItem {
  id: MakoTopLevelView;
  label: string;
  detail: string;
}

export const MAKO_DRAWER_ITEMS: MakoDrawerItem[] = [
  {
    id: "mako",
    label: "Hive",
    detail: "Main thread",
  },
  {
    id: "schedule",
    label: "Schedule",
    detail: "Agenda and calendar",
  },
];
