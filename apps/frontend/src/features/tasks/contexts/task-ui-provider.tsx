import type { TaskIteration } from '@moatless/shared/types/task'
import { createContext, type ReactNode, useContext, useMemo, useState } from 'react'

type PanelView = 'events' | 'diff' | 'files' | 'preview' | 'server'

interface ChatState {
  scrollPosition?: number
  isAtBottom: boolean
}

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
  
  // Chat state
  chatState: ChatState
  
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
  updateChatState: (updates: Partial<ChatState>) => void
  updateGitDiffState: (updates: Partial<GitDiffState>) => void
}

const TaskUIContext = createContext<TaskUIContextValue | undefined>(undefined)

interface TaskUIProviderProps {
  taskId: string
  children: ReactNode
}

function getDefaultTaskUIState(): TaskUIState {
  return {
    rightPanelView: 'events',
    selectedIteration: undefined,
    panelSizes: {
      left: 60,
      right: 40,
    },
    selectedFile: '',
    chatState: {
      isAtBottom: true,
    },
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
  
  // Get current task's state or default
  const currentState = taskStates.get(taskId) || getDefaultTaskUIState()
  
  // Update current task's state
  const updateTaskState = (updates: Partial<TaskUIState>) => {
    setTaskStates(prev => new Map(prev).set(taskId, { ...currentState, ...updates }))
  }
  
  // All actions are automatically scoped to current task
  const setRightPanelView = (view: PanelView) => 
    updateTaskState({ rightPanelView: view })
    
  const setSelectedIteration = (iteration?: TaskIteration) =>
    updateTaskState({ selectedIteration: iteration })
    
  const setPanelSizes = (sizes: { left: number; right: number }) =>
    updateTaskState({ panelSizes: sizes })
    
  const setSelectedFile = (file: string) =>
    updateTaskState({ selectedFile: file })
    
  const updateChatState = (updates: Partial<ChatState>) =>
    updateTaskState({ chatState: { ...currentState.chatState, ...updates } })
    
  const updateGitDiffState = (updates: Partial<GitDiffState>) =>
    updateTaskState({ gitDiffState: { ...currentState.gitDiffState, ...updates } })

  // Context value is stable since it only contains current task state
  const contextValue: TaskUIContextValue = useMemo(
    () => ({
      currentState,
      setRightPanelView,
      setSelectedIteration,
      setPanelSizes,
      setSelectedFile,
      updateChatState,
      updateGitDiffState,
    }),
    [currentState],
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

export function useChatState() {
  const { currentState, updateChatState } = useTaskUI()
  return { chatState: currentState.chatState, updateChatState }
}

export function useGitDiffState() {
  const { currentState, updateGitDiffState } = useTaskUI()
  return { gitDiffState: currentState.gitDiffState, updateGitDiffState }
}