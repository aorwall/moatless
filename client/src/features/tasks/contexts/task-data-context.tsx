import type { Event } from '@shared/types/event'
import type { GitDiffResponse, Task, TaskIteration } from '@shared/types/task'
import { createContext, type ReactNode, useContext, useMemo, useState } from 'react'
import { useTaskData } from '../hooks/use-task-data'
import { useTaskStreaming } from '../hooks/use-task-streaming'

interface TaskDataContextValue {
  // Data from API
  task: Task | undefined
  events: Event[]
  gitDiff: GitDiffResponse | undefined
  isLoading: boolean
  error: Error | null

  // UI State
  selectedIteration: TaskIteration | null
  rightPanelView: 'events' | 'diff' | 'server' | 'preview' | 'files'

  // Streaming state
  streamEvents: Event[]
  isStreaming: boolean
  streamingError: string | null

  // Actions
  setSelectedIteration: (iteration: TaskIteration | null) => void
  setRightPanelView: (view: 'events' | 'diff' | 'server' | 'preview' | 'files') => void
}

const TaskDataContext = createContext<TaskDataContextValue | undefined>(undefined)

interface TaskDataProviderProps {
  taskId: string
  children: ReactNode
}

/**
 * Provider that manages all task-related state in one place.
 * This includes:
 * - Task data fetched from API (task, events, gitDiff)
 * - UI state (selected iteration, panel view)
 * - Streaming events state
 *
 * All state is scoped to a specific taskId, ensuring proper isolation.
 */
export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  // Fetch task data in parallel
  const taskData = useTaskData(taskId)

  // UI State (previously in Zustand)
  const [selectedIteration, setSelectedIteration] = useState<TaskIteration | null>(null)
  const [rightPanelView, setRightPanelView] = useState<
    'events' | 'diff' | 'server' | 'preview' | 'files'
  >('events')

  // Streaming state
  const { streamEvents, isStreaming, streamingError } = useTaskStreaming(taskId)

  // Merge all events (API + streaming)
  const allEvents = useMemo(() => {
    // If we have stream events, they are the source of truth
    if (streamEvents.length > 0) {
      return streamEvents
    }
    // Otherwise use API events
    return taskData.events || []
  }, [streamEvents, taskData.events])

  const contextValue: TaskDataContextValue = useMemo(
    () => ({
      // Data
      task: taskData.task,
      events: allEvents,
      gitDiff: taskData.gitDiff,
      isLoading: taskData.isLoading,
      error: taskData.error,

      // UI State
      selectedIteration,
      rightPanelView,

      // Streaming
      streamEvents,
      isStreaming,
      streamingError,

      // Actions
      setSelectedIteration,
      setRightPanelView,
    }),
    [
      taskData,
      selectedIteration,
      rightPanelView,
      streamEvents,
      isStreaming,
      streamingError,
      allEvents,
    ],
  )

  return <TaskDataContext.Provider value={contextValue}>{children}</TaskDataContext.Provider>
}

/**
 * Hook to access all task data and state from context.
 * Must be used within TaskDataProvider.
 */
export function useTaskDataContext() {
  const context = useContext(TaskDataContext)
  if (!context) {
    throw new Error('useTaskDataContext must be used within TaskDataProvider')
  }
  return context
}

// Convenience hooks for specific parts of the context
export function useSelectedIteration() {
  const { selectedIteration } = useTaskDataContext()
  return selectedIteration
}

export function useRightPanelView() {
  const { rightPanelView } = useTaskDataContext()
  return rightPanelView
}
