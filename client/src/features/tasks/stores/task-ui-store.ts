import type { TaskIteration } from '@shared/types/task'
import { create } from 'zustand'
import { subscribeWithSelector } from 'zustand/middleware'

interface TaskUIState {
  // UI state
  selectedIteration: TaskIteration | null
  rightPanelView: 'events' | 'diff' | 'server' | 'preview' | 'files'
  selectedTaskId: string | null

  // Actions
  setSelectedIteration: (iteration: TaskIteration | null) => void
  setRightPanelView: (view: 'events' | 'diff' | 'server' | 'preview' | 'files') => void
  setSelectedTaskId: (taskId: string | null) => void
  resetUIState: () => void
}

const initialState = {
  selectedIteration: null,
  rightPanelView: 'events' as const,
  selectedTaskId: null,
}

export const useTaskUIStore = create<TaskUIState>()(
  subscribeWithSelector((set) => ({
    ...initialState,

    // UI state setters
    setSelectedIteration: (iteration) => set({ selectedIteration: iteration }),
    setRightPanelView: (view) => set({ rightPanelView: view }),
    setSelectedTaskId: (taskId) => set({ selectedTaskId: taskId }),

    // Reset all UI state
    resetUIState: () => set(initialState),
  })),
)

// Selectors for common use cases
export const useSelectedIteration = () => useTaskUIStore((state) => state.selectedIteration)
export const useRightPanelView = () => useTaskUIStore((state) => state.rightPanelView)
