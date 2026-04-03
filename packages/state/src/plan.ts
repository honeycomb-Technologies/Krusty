import { create } from 'zustand';

export interface PlanItem {
  id: string;
  content: string;
  completed: boolean;
}

export interface PlanStoreState {
  items: PlanItem[];
  isVisible: boolean;
  addItem: (content: string) => string;
  toggleItem: (id: string) => void;
  removeItem: (id: string) => void;
  clear: () => void;
  setVisible: (visible: boolean) => void;
  setItems: (items: Array<{ content: string; completed?: boolean }>) => void;
}

let idCounter = 0;

export function createPlanStore() {
  return create<PlanStoreState>((set) => ({
    items: [],
    isVisible: false,

    addItem: (content: string) => {
      const id = `plan-${++idCounter}`;
      set((s) => ({
        items: [...s.items, { id, content, completed: false }],
        isVisible: true,
      }));
      return id;
    },

    toggleItem: (id: string) => {
      set((s) => ({
        items: s.items.map((item) =>
          item.id === id ? { ...item, completed: !item.completed } : item
        ),
      }));
    },

    removeItem: (id: string) => {
      set((s) => ({
        items: s.items.filter((item) => item.id !== id),
      }));
    },

    clear: () => {
      set({ items: [], isVisible: false });
    },

    setVisible: (visible: boolean) => {
      set({ isVisible: visible });
    },

    setItems: (items: Array<{ content: string; completed?: boolean }>) => {
      set({
        items: items.map((item) => ({
          id: `plan-${++idCounter}`,
          content: item.content,
          completed: item.completed ?? false,
        })),
        isVisible: items.length > 0,
      });
    },
  }));
}
