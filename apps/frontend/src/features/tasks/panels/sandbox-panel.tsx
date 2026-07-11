import { ErrorBoundary } from '@/lib/components/error-boundary'
import { SandboxWorkspacePanel } from '../components/sandbox-workspace-panel'
import { useSandboxEvents, useTaskId } from '../contexts/task-providers'
import { useSandboxStatus } from '../hooks/use-sandbox-status'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useTaskView } from '../hooks/use-task-view'

/**
 * Sandbox controls view: the control panel plus the shared lifecycle-events
 * console. Self-contained — reads servers and lifecycle events from context.
 */
export function SandboxPanel() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ servers })
  const sandboxEvents = useSandboxEvents()

  return (
    <ErrorBoundary componentName="Sandbox Controls">
      <SandboxWorkspacePanel
        view={view}
        onViewChange={setView}
        servers={servers}
        sandboxEvents={sandboxEvents}
      />
    </ErrorBoundary>
  )
}
