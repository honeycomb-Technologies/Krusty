import type { PlanItem as ApiPlanItem, WorkflowSnapshot } from '@krusty/api';
import { create } from 'zustand';

export interface PlanItem {
  id: string;
  displayKey?: string;
  content: string;
  completed: boolean;
  status?: string;
}

export interface PlanStoreState {
  workflow: WorkflowSnapshot | null;
  items: PlanItem[];
  isVisible: boolean;
  pendingRevision: number | null;
  setWorkflow: (workflow: WorkflowSnapshot | null) => void;
  noteWorkflowRevision: (aggregateRevision: number) => void;
  setVisible: (visible: boolean) => void;
  /** Temporary read-only adapter for pre-Workflow-v2 PlanUpdate events. */
  setItems: (items: ApiPlanItem[]) => void;
}

function projectWorkflowItems(workflow: WorkflowSnapshot): PlanItem[] {
  return workflow.steps.map((step) => ({
    id: step.id,
    displayKey: step.display_key,
    content: step.description,
    completed: step.status === 'completed' || step.status === 'skipped',
    status: step.status,
  }));
}

export function createPlanStore() {
  return create<PlanStoreState>((set) => ({
    workflow: null,
    items: [],
    isVisible: false,
    pendingRevision: null,

    setWorkflow: (workflow) => {
      set((state) => ({
        workflow,
        items: workflow ? projectWorkflowItems(workflow) : [],
        isVisible: workflow ? state.isVisible || workflow.goal.status !== 'cancelled' : false,
        pendingRevision: null,
      }));
    },

    noteWorkflowRevision: (aggregateRevision) => {
      set((state) => ({
        pendingRevision:
          aggregateRevision > (state.workflow?.aggregate_revision ?? 0)
            ? aggregateRevision
            : state.pendingRevision,
      }));
    },

    setVisible: (visible) => {
      set({ isVisible: visible });
    },

    setItems: (items) => {
      set((state) => {
        if (state.workflow) return state;
        return {
          items: items.map((item, index) => ({
            id: item.id ?? `legacy:${index}:${item.content}`,
            content: item.content,
            completed: item.completed ?? false,
          })),
          isVisible: items.length > 0,
        };
      });
    },
  }));
}
