import type { Event } from '@shared/types/event'
import { createContext, useContext, type ReactNode } from 'react'
import { useTaskEventsWithStreaming } from '../hooks/use-task-events'

interface TaskEventsContextValue {
  events: Event[]
  isLoading: boolean
  error: Error | null
  isStreaming: boolean
}

const TaskEventsContext = createContext<TaskEventsContextValue | undefined>(undefined)

export function TaskEventsProvider({ taskId, children }: { taskId: string; children: ReactNode }) {
  const eventsData = useTaskEventsWithStreaming(taskId)

  return <TaskEventsContext.Provider value={eventsData}>{children}</TaskEventsContext.Provider>
}

export function useTaskEventsContext() {
  const context = useContext(TaskEventsContext)
  if (!context) {
    throw new Error('useTaskEventsContext must be used within TaskEventsProvider')
  }
  return context
}

// Helper hook to conditionally use context or direct hook
export function useTaskEvents(taskId: string) {
  try {
    // Try to use context first if available
    return useTaskEventsContext()
  } catch {
    // Fall back to direct hook if not in context
    return useTaskEventsWithStreaming(taskId)
  }
}