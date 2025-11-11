import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskStatus } from '@/features/tasks/contexts/task-providers'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'

interface FileExplorerHeaderProps {
  serverManagement?: boolean
}

export function FileExplorerHeader({ serverManagement }: FileExplorerHeaderProps) {
  const { status } = useTaskStatus()
  const { view, setView } = useTaskView({ serverManagement })

  // Map TaskView to WorkspaceHeader's expected view type
  const activeView = view === 'files' || view === 'preview' ? view : 'files'
  const handleViewChange = (newView: 'files' | 'preview') => {
    setView(newView)
  }

  if (!status) {
    return (
      <WorkspaceHeader
        activeView={activeView}
        onViewChange={handleViewChange}
        showToggle={serverManagement}
      />
    )
  }

  return (
    <WorkspaceHeader
      activeView={activeView}
      onViewChange={handleViewChange}
      showToggle={serverManagement}
    />
  )
}
