import { LazyFileExplorer } from '@/features/files'
import { SandboxStatusOverlay } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { WorkspaceHeader } from '../components/workspace-header'
import { useTaskId } from '../contexts/task-providers'
import { useSandboxRetry, useSandboxStatus } from '../hooks/use-sandbox-status'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useTaskView } from '../hooks/use-task-view'

/**
 * Files view: the file explorer, gated behind a sandbox-readiness overlay.
 * Self-contained — fetches its own sandbox status and retry mutation.
 */
export function FilesPanel() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ servers })
  const retryMutation = useSandboxRetry(taskId)

  const isSandboxReady = sandboxStatus?.environmentStatus === 'ready'

  return (
    <ErrorBoundary componentName="File Explorer">
      <div className="h-full flex flex-col">
        <WorkspaceHeader activeView={view} onViewChange={setView} servers={servers} />
        <div className="flex-1 min-h-0 relative">
          {isSandboxReady ? (
            <LazyFileExplorer taskId={taskId} />
          ) : (
            sandboxStatus?.environmentStatus && (
              <SandboxStatusOverlay
                environmentStatus={sandboxStatus.environmentStatus}
                environmentError={sandboxStatus.environmentError ?? undefined}
                onRetry={() => retryMutation.mutate()}
                isRetrying={retryMutation.isPending}
                retryError={retryMutation.error}
              />
            )
          )}
        </div>
      </div>
    </ErrorBoundary>
  )
}
