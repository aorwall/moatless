import type { ServerEntry } from '@/features/preview/hooks/use-servers'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'

interface FileExplorerHeaderProps {
  serverManagement?: boolean
  servers?: ServerEntry[]
}

export function FileExplorerHeader({ serverManagement, servers }: FileExplorerHeaderProps) {
  const { view, setView } = useTaskView({ serverManagement })

  // Map TaskView to WorkspaceHeader's expected view type
  const activeView = view === 'files' || view === 'preview' || view === 'diff' ? view : 'files'
  const handleViewChange = (newView: 'files' | 'preview' | 'diff') => {
    setView(newView)
  }

  return (
    <WorkspaceHeader
      activeView={activeView}
      onViewChange={handleViewChange}
      showToggle={serverManagement}
      servers={servers}
    />
  )
}
