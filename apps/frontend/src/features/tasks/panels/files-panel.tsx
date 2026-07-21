import { useNavigate, useSearch } from '@tanstack/react-router'
import { useEffect } from 'react'
import { FilesWorkspaceControls, LazyFileExplorer } from '@/features/files'
import { SandboxStatusOverlay } from '@/features/preview'
import { ViewTabs } from '@/features/preview/components/view-tabs'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { WorkspaceHeader } from '@/lib/layout/workspace-header'
import { useTaskId } from '../contexts/task-providers'
import { useSandboxRetry, useSandboxStatus } from '../hooks/use-sandbox-status'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useTaskView } from '../hooks/use-task-view'
import type { TaskSearch } from '../types/task-search-schema'

/**
 * Files view: the file explorer, gated behind a sandbox-readiness overlay.
 * Self-contained — fetches its own sandbox status and retry mutation.
 */
export function FilesPanel() {
  const navigate = useNavigate()
  const search = useSearch({ strict: false }) as TaskSearch
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ servers })
  const retryMutation = useSandboxRetry(taskId)
  const isMobile = useIsMobile()

  const isSandboxReady = sandboxStatus?.sandboxStatus === 'ready'

  useEffect(() => {
    if (search.path === undefined && search.device === undefined) {
      return
    }

    navigate({
      to: '.',
      search: (prev) => {
        const { path: _path, device: _device, ...rest } = prev as TaskSearch
        return rest
      },
      replace: true,
    }).catch(() => {
      /* ignore: URL cleanup */
    })
  }, [navigate, search.path, search.device])

  return (
    <ErrorBoundary componentName="File Explorer">
      <div className="h-full flex flex-col">
        {/* On mobile the header controls live in the floating overlay's actions
            menu (see mobile-task-layout), so the panel goes header-less. */}
        {!isMobile && (
          <WorkspaceHeader
            leading={<ViewTabs servers={servers ?? []} activeView={view} onViewChange={setView} />}
            centerContent={<FilesWorkspaceControls taskId={taskId} />}
          />
        )}
        <div className="flex-1 min-h-0 relative">
          {isSandboxReady ? (
            <LazyFileExplorer taskId={taskId} />
          ) : (
            sandboxStatus?.sandboxStatus && (
              <SandboxStatusOverlay
                sandboxStatus={sandboxStatus.sandboxStatus}
                sandboxError={sandboxStatus.sandboxError ?? undefined}
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
