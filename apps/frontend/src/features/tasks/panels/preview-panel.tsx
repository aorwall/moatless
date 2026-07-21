import { toast } from 'sonner'
import { Preview } from '@/features/preview'
import type { PreviewPublish } from '@/features/preview/types'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { getErrorMessage } from '@/lib/utils/error'
import {
  useComposer,
  usePanelControls,
  useSandboxEvents,
  useStartupEvents,
  useTaskId,
} from '../contexts/task-providers'
import { useSandboxStatus } from '../hooks/use-sandbox-status'
import { useTaskDetailQuery } from '../hooks/use-task-queries'
import { useTaskView } from '../hooks/use-task-view'
import { useDeployTask } from '../hooks/use-tasks'

interface PreviewPanelProps {
  /** Kiosk variant: hide the preview header and console. */
  kiosk?: boolean
}

/**
 * Server/preview view: the preview iframe and its overlays. Self-contained —
 * joins sandbox status, lifecycle events, composer, and panel controls from
 * context and passes them into the presentational Preview component.
 */
export function PreviewPanel({ kiosk = false }: PreviewPanelProps) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const task = taskQuery.data
  const { data: sandboxStatus } = useSandboxStatus(taskId, task)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ servers })

  const composer = useComposer()
  const controls = usePanelControls()
  const sandboxEvents = useSandboxEvents()
  const startupEvents = useStartupEvents()
  const isMobile = useIsMobile()

  const activeServerName = view === 'chat' ? undefined : view

  // Publish (deploy) state — composed here and passed into the presentational
  // Preview header so `features/preview` doesn't reach into task/repository data.
  const deployTask = useDeployTask(taskId, {
    onSuccess: (data) => {
      if (data.success && data.siteUrl) {
        toast.success('Site published successfully!', {
          description: `Your site is live at ${data.siteUrl}`,
          action: {
            label: 'Open Site',
            onClick: () => window.open(data.siteUrl, '_blank', 'noopener,noreferrer'),
          },
        })
      } else if (data.error) {
        toast.error('Failed to publish site', { description: data.error })
      }
    },
    onError: (error) => {
      toast.error('Failed to publish site', { description: getErrorMessage(error) })
    },
  })
  const publish: PreviewPublish = {
    canPublish: Boolean(task?.vibe),
    siteUrl: task?.siteUrl ?? undefined,
    onPublish: () => deployTask.mutate(),
    isPending: deployTask.isPending,
  }

  return (
    <ErrorBoundary componentName="Preview">
      <Preview
        taskId={taskId}
        sandboxStatus={sandboxStatus?.sandboxStatus}
        sandboxError={sandboxStatus?.sandboxError ?? undefined}
        startupEvents={startupEvents}
        onToggleElementContext={composer.toggleElementContext}
        onNavigationReady={controls.registerPreviewNavigate}
        onReloadReady={controls.registerPreviewReload}
        onChatToggleRequest={() => controls.toggleChat()}
        servers={servers}
        activeServer={activeServerName}
        view={view}
        onViewChange={setView}
        publish={publish}
        resources={sandboxStatus?.resources ?? undefined}
        sandboxEvents={sandboxEvents}
        hideHeader={kiosk || isMobile}
        hideConsole={kiosk}
      />
    </ErrorBoundary>
  )
}
