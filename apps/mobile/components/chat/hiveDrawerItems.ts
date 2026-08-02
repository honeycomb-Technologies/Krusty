import type { HiveTopLevelView } from "../hive/types";

export interface HiveDrawerItem {
  id: HiveTopLevelView;
  label: string;
  detail: string;
}

export const HIVE_DRAWER_ITEMS: HiveDrawerItem[] = [
  {
    id: "hive",
    label: "Hive",
    detail: "Main thread",
  },
  {
    id: "schedule",
    label: "Schedule",
    detail: "Agenda and calendar",
  },
];
