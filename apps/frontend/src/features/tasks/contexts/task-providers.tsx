import type { ReactNode } from 'react'
import { TaskDataProvider } from './task-data-provider'
import { TaskUIProvider } from './task-ui-provider'

interface TaskProvidersProps {
  taskId: string
  children: ReactNode
}

/**
 * Composed provider that includes task data and UI providers.
 * TaskDataProvider handles events, workspace status, and task metadata.
 * TaskUIProvider handles local UI state (panel views, selections).
 */
export function TaskProviders({ taskId, children }: TaskProvidersProps) {
  return (
    <TaskUIProvider taskId={taskId}>
      <TaskDataProvider taskId={taskId}>{children}</TaskDataProvider>
    </TaskUIProvider>
  )
}

// Export all the individual providers and hooks
export {
  TaskDataProvider,
  useEventStream,
  useGitDiff,
  useGitDiffWithLoading,
  useLatestContextTokens,
  useStreamingStatus,
  useTaskData,
  useTaskDuration,
  // Event hooks (now in TaskDataProvider)
  useTaskEvents,
  useTaskIterations,
  useTaskLoadingError,
  useTaskMetadata,
  useTaskStatus,
} from './task-data-provider'
export {
  TaskUIProvider,
  useGitDiffState,
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useSelectedIteration,
  useTaskUI,
} from './task-ui-provider'
