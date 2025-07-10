import type { ReactNode } from 'react'
import { TaskDataProvider } from './task-data-provider'
import { TaskEventsProvider } from './task-events-provider'
import { TaskUIProvider } from './task-ui-provider'

interface TaskProvidersProps {
  taskId: string
  children: ReactNode
}

/**
 * Composed provider that includes all three task providers.
 * This implements the Tri-Context Pattern for optimal performance.
 */
export function TaskProviders({ taskId, children }: TaskProvidersProps) {
  return (
    <TaskUIProvider taskId={taskId}>
      <TaskDataProvider taskId={taskId}>
        <TaskEventsProvider taskId={taskId}>
          {children}
        </TaskEventsProvider>
      </TaskDataProvider>
    </TaskUIProvider>
  )
}

// Export all the individual providers and hooks
export {
  TaskDataProvider,
  useGitDiff,
  useGitDiffWithLoading,
  useTaskData,
  useTaskDuration,
  useTaskIterations,
  useTaskLoadingError,
  useTaskMetadata,
  useTaskStatus,
  useWorkspaceStatus,
  useWorkspaceStatusWithLoading,
} from './task-data-provider'
export {
  TaskEventsProvider,
  useEventStream,
  useStreamingStatus,
  useTaskEvents,
  useLatestContextTokens,
} from './task-events-provider'
export {
  TaskUIProvider,
  useChatState,
  useGitDiffState,
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useSelectedIteration,
  useTaskUI,
} from './task-ui-provider'
