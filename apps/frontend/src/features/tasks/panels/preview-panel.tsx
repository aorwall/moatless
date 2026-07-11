import { Preview } from '@/features/preview'
import { ErrorBoundary } from '@/lib/components/error-boundary'
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
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view } = useTaskView({ servers })

  const composer = useComposer()
  const controls = usePanelControls()
  const sandboxEvents = useSandboxEvents()
  const startupEvents = useStartupEvents()

  const activeServerName = view === 'chat' ? undefined : view

  return (
    <ErrorBoundary componentName="Preview">
      <Preview
        taskId={taskId}
        environmentStatus={sandboxStatus?.environmentStatus}
        environmentError={sandboxStatus?.environmentError ?? undefined}
        startupEvents={startupEvents}
        onToggleElementContext={composer.toggleElementContext}
        onNavigationReady={controls.registerPreviewNavigate}
        onTroubleshootRequest={() => controls.executeCommand('troubleshoot-preview')}
        onChatToggleRequest={() => controls.toggleChat()}
        servers={servers}
        activeServer={activeServerName}
        resources={sandboxStatus?.resources ?? undefined}
        sandboxEvents={sandboxEvents}
        hideHeader={kiosk}
        hideConsole={kiosk}
      />
    </ErrorBoundary>
  )
}
