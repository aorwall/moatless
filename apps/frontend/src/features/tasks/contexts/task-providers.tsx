import type { ReactNode } from 'react'
import { SandboxEventsProvider } from './sandbox-events-provider'
import { TaskComposerProvider } from './task-composer-provider'
import { TaskDataProvider } from './task-data-provider'
import { TaskPanelControlsProvider } from './task-panel-controls'
import { TaskUIProvider } from './task-ui-provider'

interface TaskProvidersProps {
  taskId: string
  children: ReactNode
}

/**
 * Composed provider stack for the task page.
 *
 * - TaskDataProvider: SSE stream buffer, task-scoped identity.
 * - SandboxEventsProvider: accumulated sandbox/server + startup events.
 * - TaskComposerProvider: chat composer context items.
 * - TaskPanelControlsProvider: semantic cross-panel actions.
 * - TaskUIProvider: local UI state (panel sizes, selections).
 *
 * All state that must outlive a mobile↔desktop layout swap lives here, above
 * the layouts.
 */
export function TaskProviders({ taskId, children }: TaskProvidersProps) {
  return (
    <TaskDataProvider key={taskId} taskId={taskId}>
      <SandboxEventsProvider>
        <TaskComposerProvider>
          <TaskPanelControlsProvider>
            <TaskUIProvider taskId={taskId}>{children}</TaskUIProvider>
          </TaskPanelControlsProvider>
        </TaskComposerProvider>
      </SandboxEventsProvider>
    </TaskDataProvider>
  )
}

export { useIsAgentRunning, useTaskMetadata, useTaskStatus } from '../hooks/use-task-context'
export {
  SandboxEventsProvider,
  useSandboxEvents,
  useStartupEvents,
} from './sandbox-events-provider'
export { TaskComposerProvider, useComposer } from './task-composer-provider'
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
export { TaskPanelControlsProvider, usePanelControls } from './task-panel-controls'
export { TaskUIProvider, usePanelSizes, useTaskUI } from './task-ui-provider'
