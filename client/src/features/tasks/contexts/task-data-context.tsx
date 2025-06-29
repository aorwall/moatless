import type { Event } from '@shared/types/event'
import type { GitDiffResponse, Task, TaskIteration } from '@shared/types/task'
import type { WorkspaceStatus } from '@shared/types/workspace'
import { useQueryClient } from '@tanstack/react-query'
import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from 'react'
import { queryKeys } from '@/lib/api/query-keys'
import { useTaskData } from '../hooks/use-task-data'

interface TaskContextValue {
  // Remote state (from API)
  task?: Task
  events: Event[]
  iterations: TaskIteration[]
  gitDiff?: GitDiffResponse
  workspaceStatus?: WorkspaceStatus

  // Loading & error states
  isLoading: boolean
  error: Error | null

  // Real-time state
  isStreaming: boolean
  streamEvents: Event[]

  // UI state (local)
  rightPanelView: 'events' | 'diff' | 'files' | 'preview' | 'server'
  selectedIteration?: TaskIteration

  // Actions
  setRightPanelView: (view: 'events' | 'diff' | 'files' | 'preview' | 'server') => void
  setSelectedIteration: (iteration?: TaskIteration) => void
}

const TaskContext = createContext<TaskContextValue | undefined>(undefined)

interface TaskProviderProps {
  taskId: string
  children: ReactNode
}

// Streaming hook options interface
interface UseTaskStreamingOptions {
  enabled?: boolean
  onEvent?: (event: Event) => void
  onError?: (error: Error) => void
  onConnect?: () => void
  onDisconnect?: () => void
}

// Simplified streaming hook integrated into provider
function useTaskStreaming(_taskId: string, _options: UseTaskStreamingOptions = {}) {
  const [isConnected] = useState(false)

  // For now, return mock values - will implement actual SSE later
  return { isConnected }
}

/**
 * TaskProvider that manages all task-related state in one place.
 * This is the single source of truth for all task data and UI state.
 *
 * Provides:
 * - Task data fetched from API (task, events, iterations, gitDiff)
 * - Real-time streaming events via SSE
 * - UI state (selected iteration, panel view)
 * - Loading and error states
 *
 * All components should consume data through useTaskContext() instead of
 * making direct API calls.
 */
export function TaskProvider({ taskId, children }: TaskProviderProps) {
  const queryClient = useQueryClient()

  // Single data fetch - parallel queries
  const taskData = useTaskData(taskId)

  // UI State (local to this task instance)
  const [rightPanelView, setRightPanelView] = useState<
    'events' | 'diff' | 'files' | 'preview' | 'server'
  >('events')
  const [selectedIteration, setSelectedIteration] = useState<TaskIteration>()

  // Streaming state
  const [streamEvents, setStreamEvents] = useState<Event[]>([])
  const [isStreaming] = useState(false)

  // Streaming event handler
  const handleStreamEvent = useCallback(
    (event: Event) => {
      setStreamEvents((prev) => {
        // Prevent duplicates
        if (prev.some((e) => e.eventId === event.eventId)) {
          return prev
        }
        return [...prev, event]
      })

      // Update React Query cache with new events
      queryClient.setQueryData(queryKeys.taskEvents(taskId), (old: Event[] = []) => {
        // Prevent duplicates in cache too
        if (old.some((e) => e.eventId === event.eventId)) {
          return old
        }
        return [...old, event]
      })

      // For now, we'll let the regular polling handle status updates
      // TODO: Implement proper status change event handling when available
    },
    [queryClient, taskId],
  )

  // Streaming connection (stateless, just reports events)
  const { isConnected } = useTaskStreaming(taskId, {
    enabled: !!taskId,
    onEvent: handleStreamEvent,
    onConnect: () => setIsStreaming(true),
    onDisconnect: () => setIsStreaming(false),
  })

  // Merge all events (API + streaming)
  const allEvents = useMemo(() => {
    const eventMap = new Map()

    // Add API events first

    ;(taskData.events || []).forEach((event) => {
      eventMap.set(event.eventId, event)
    })

    // Add stream events (will overwrite any duplicates)
    streamEvents.forEach((event) => {
      eventMap.set(event.eventId, event)
    })

    return Array.from(eventMap.values()).sort((a, b) => {
      const aTime = a.timestamp ? new Date(a.timestamp).getTime() : 0
      const bTime = b.timestamp ? new Date(b.timestamp).getTime() : 0
      return aTime - bTime
    })
  }, [taskData.events, streamEvents])

  const contextValue: TaskContextValue = useMemo(
    () => ({
      // Data
      task: taskData.task,
      events: allEvents,
      iterations: taskData.iterations,
      gitDiff: taskData.gitDiff,
      workspaceStatus: undefined, // TODO: Will be added when workspace status is implemented
      isLoading: taskData.isLoading,
      error: taskData.error,

      // Streaming
      isStreaming: isConnected,
      streamEvents,

      // UI State
      rightPanelView,
      selectedIteration,
      setRightPanelView,
      setSelectedIteration,
    }),
    [taskData, allEvents, isConnected, streamEvents, rightPanelView, selectedIteration],
  )

  return <TaskContext.Provider value={contextValue}>{children}</TaskContext.Provider>
}

/**
 * Hook to access all task data and state from context.
 * Must be used within TaskProvider.
 *
 * This replaces all direct useTask, useTaskEvents, etc. calls.
 */
export function useTaskContext() {
  const context = useContext(TaskContext)
  if (!context) {
    throw new Error('useTaskContext must be used within TaskProvider')
  }
  return context
}

// Legacy export for backward compatibility
export const TaskDataProvider = TaskProvider
export const useTaskDataContext = useTaskContext

// Convenience hooks for specific parts of the context
export function useSelectedIteration() {
  const { selectedIteration } = useTaskContext()
  return selectedIteration
}

export function useRightPanelView() {
  const { rightPanelView } = useTaskContext()
  return rightPanelView
}
