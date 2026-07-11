import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { useTaskId } from '../contexts/task-providers'
import { useSandboxStatus } from '../hooks/use-sandbox-status'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useTaskView } from '../hooks/use-task-view'
import { SANDBOX_VIEW } from '../types/view-registry'
import { FilesPanel } from './files-panel'
import { PreviewPanel } from './preview-panel'
import { SandboxPanel } from './sandbox-panel'

interface WorkspacePanelProps {
  /** Kiosk variant, forwarded to the preview panel. */
  kiosk?: boolean
}

/**
 * The non-chat side of the task page: selects which content panel to render
 * based on the current `?view=`. Content only — layouts own the arrangement.
 * Gates rendering on sandbox status so the correct default view is chosen.
 */
export function WorkspacePanel({ kiosk = false }: WorkspacePanelProps) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const sandboxStatusQuery = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatusQuery.data?.servers
  const { view } = useTaskView({ servers })

  if (sandboxStatusQuery.isLoading) {
    return (
      <div className="p-4 space-y-4">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-32 w-full" />
      </div>
    )
  }

  if (sandboxStatusQuery.error) {
    return <ErrorAlert error={sandboxStatusQuery.error} />
  }

  if (view === 'files') {
    return <FilesPanel />
  }

  if (view === SANDBOX_VIEW) {
    return <SandboxPanel />
  }

  return <PreviewPanel kiosk={kiosk} />
}
