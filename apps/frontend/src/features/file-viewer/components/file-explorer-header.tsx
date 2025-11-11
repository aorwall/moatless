import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskStatus } from '@/features/tasks/contexts/task-providers'
import { usePanelView } from '@/features/tasks/contexts/task-ui-provider'

interface FileExplorerHeaderProps {
  serverManagement?: boolean
  activeView?: 'files' | 'preview'
  onViewChange?: (view: 'files' | 'preview') => void
}

export function FileExplorerHeader({
  serverManagement,
  activeView,
  onViewChange,
}: FileExplorerHeaderProps) {
  const { status } = useTaskStatus()
  const panelView = usePanelView(serverManagement)

  // Use provided props or fall back to context
  const currentView = activeView ?? panelView.rightPanelView
  const handleViewChange = onViewChange ?? panelView.setRightPanelView

  if (!status) {
    return (
      <WorkspaceHeader
        showToggle={serverManagement}
        activeView={currentView}
        onViewChange={handleViewChange}
      />
    )
  }

  return (
    <WorkspaceHeader
      showToggle={serverManagement}
      activeView={currentView}
      onViewChange={handleViewChange}
    />
  )
}
