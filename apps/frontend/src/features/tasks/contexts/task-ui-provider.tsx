import type { TaskIteration } from '@moatless/types'
import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from 'react'

type PanelView = 'files' | 'preview'

interface GitDiffState {
  expandedFiles: Set<string>
  selectedFileKey?: string
  expandedPaths: Set<string>
}

interface TaskUIState {
  // Panel and layout state
  rightPanelView: PanelView
  selectedIteration: TaskIteration | undefined
  panelSizes: {
    left: number
    right: number
  }
  selectedFile: string

  // Git diff state
  gitDiffState: GitDiffState
}

interface TaskUIContextValue {
  // Current task's state
  currentState: TaskUIState

  // Actions that automatically scope to current task
  setRightPanelView: (view: PanelView) => void
  setSelectedIteration: (iteration?: TaskIteration) => void
  setPanelSizes: (sizes: { left: number; right: number }) => void
  setSelectedFile: (file: string) => void
  updateGitDiffState: (updates: Partial<GitDiffState>) => void
}

const TaskUIContext = createContext<TaskUIContextValue | undefined>(undefined)

interface TaskUIProviderProps {
  taskId: string
  children: ReactNode
}

function getDefaultTaskUIState(): TaskUIState {
  return {
    rightPanelView: 'files',
    selectedIteration: undefined,
    panelSizes: {
      left: 60,
      right: 40,
    },
    selectedFile: '',
    gitDiffState: {
      expandedFiles: new Set(),
      expandedPaths: new Set(),
    },
  }
}

/**
 * TaskUIProvider manages pure UI state with complete task isolation.
 * Each task has its own UI state that persists when switching between tasks.
 */
export function TaskUIProvider({ taskId, children }: TaskUIProviderProps) {
  // Map of taskId -> TaskUIState for complete isolation
  const [taskStates, setTaskStates] = useState<Map<string, TaskUIState>>(new Map())

  // Get current task's state or default - memoize to prevent changes on every render
  const currentState = useMemo(
    () => taskStates.get(taskId) || getDefaultTaskUIState(),
    [taskStates, taskId],
  )

  // All actions are automatically scoped to current task using useCallback
  const setRightPanelView = useCallback(
    (view: PanelView) => {
      setTaskStates((prev) => {
        const newMap = new Map(prev)
        const state = newMap.get(taskId) || getDefaultTaskUIState()
        newMap.set(taskId, { ...state, rightPanelView: view })
        return newMap
      })
    },
    [taskId],
  )

  const setSelectedIteration = useCallback(
    (iteration?: TaskIteration) => {
      setTaskStates((prev) => {
        const newMap = new Map(prev)
        const state = newMap.get(taskId) || getDefaultTaskUIState()
        newMap.set(taskId, { ...state, selectedIteration: iteration })
        return newMap
      })
    },
    [taskId],
  )

  const setPanelSizes = useCallback(
    (sizes: { left: number; right: number }) => {
      setTaskStates((prev) => {
        const newMap = new Map(prev)
        const state = newMap.get(taskId) || getDefaultTaskUIState()
        newMap.set(taskId, { ...state, panelSizes: sizes })
        return newMap
      })
    },
    [taskId],
  )

  const setSelectedFile = useCallback(
    (file: string) => {
      setTaskStates((prev) => {
        const newMap = new Map(prev)
        const state = newMap.get(taskId) || getDefaultTaskUIState()
        newMap.set(taskId, { ...state, selectedFile: file })
        return newMap
      })
    },
    [taskId],
  )

  const updateGitDiffState = useCallback(
    (updates: Partial<GitDiffState>) => {
      setTaskStates((prev) => {
        const newMap = new Map(prev)
        const state = newMap.get(taskId) || getDefaultTaskUIState()
        newMap.set(taskId, { ...state, gitDiffState: { ...state.gitDiffState, ...updates } })
        return newMap
      })
    },
    [taskId],
  )

  // Context value is stable with memoized callbacks
  const contextValue: TaskUIContextValue = useMemo(
    () => ({
      currentState,
      setRightPanelView,
      setSelectedIteration,
      setPanelSizes,
      setSelectedFile,
      updateGitDiffState,
    }),
    // Include all dependencies - callbacks are stable
    [
      currentState,
      setRightPanelView,
      setSelectedIteration,
      setPanelSizes,
      setSelectedFile,
      updateGitDiffState,
    ],
  )

  return <TaskUIContext.Provider value={contextValue}>{children}</TaskUIContext.Provider>
}

/**
 * Hook to access UI state.
 * This never re-renders due to server data changes.
 */
export function useTaskUI() {
  const context = useContext(TaskUIContext)
  if (!context) {
    throw new Error('useTaskUI must be used within TaskUIProvider')
  }
  return context
}

// Specialized hooks for specific UI state
export function usePanelView() {
  const { currentState, setRightPanelView } = useTaskUI()
  return { rightPanelView: currentState.rightPanelView, setRightPanelView }
}

export function useSelectedIteration() {
  const { currentState, setSelectedIteration } = useTaskUI()
  return { selectedIteration: currentState.selectedIteration, setSelectedIteration }
}

export function usePanelSizes() {
  const { currentState, setPanelSizes } = useTaskUI()
  return { panelSizes: currentState.panelSizes, setPanelSizes }
}

export function useSelectedFile() {
  const { currentState, setSelectedFile } = useTaskUI()
  return { selectedFile: currentState.selectedFile, setSelectedFile }
}

export function useGitDiffState() {
  const { currentState, updateGitDiffState } = useTaskUI()
  return { gitDiffState: currentState.gitDiffState, updateGitDiffState }
}
