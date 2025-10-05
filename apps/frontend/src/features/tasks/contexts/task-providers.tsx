import type { ReactNode } from 'react'
import { useTaskMetadata } from '../hooks/use-task-context'
import { TaskDataProvider } from './task-data-provider'
import { TaskUIProvider } from './task-ui-provider'

interface TaskProvidersProps {
  taskId: string
  children: ReactNode
}

/**
 * Internal component that has access to task data and can pass vibe to TaskUIProvider
 */
function TaskProvidersInternal({ taskId, children }: { taskId: string; children: ReactNode }) {
  const { vibe } = useTaskMetadata()
  return (
    <TaskUIProvider taskId={taskId} vibe={vibe}>
      {children}
    </TaskUIProvider>
  )
}

/**
 * Composed provider that includes task data and UI providers.
 * TaskDataProvider handles events, workspace status, and task metadata.
 * TaskUIProvider handles local UI state (panel views, selections).
 */
export function TaskProviders({ taskId, children }: TaskProvidersProps) {
  return (
    <TaskDataProvider taskId={taskId}>
      <TaskProvidersInternal taskId={taskId}>{children}</TaskProvidersInternal>
    </TaskDataProvider>
  )
}

export {
  useGitDiff,
  useGitDiffWithLoading,
  useIsAgentRunning,
  useTaskMetadata,
  useTaskStatus,
} from '../hooks/use-task-context'
// Export all the individual providers and hooks
export {
  TaskDataProvider,
  useEventStream,
  useLatestContextTokens,
  useStreamingStatus,
  useTaskEvents,
  useTaskId,
  useTaskStreamingState,
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
