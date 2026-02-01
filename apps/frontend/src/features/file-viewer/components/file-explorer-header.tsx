import type { ServerEntry } from '@/features/preview/hooks/use-servers'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'

interface FileExplorerHeaderProps {
  serverManagement?: boolean
  servers?: ServerEntry[]
}

export function FileExplorerHeader({ serverManagement, servers }: FileExplorerHeaderProps) {
  const { view, setView } = useTaskView({ serverManagement })

  return (
    <WorkspaceHeader
      activeView={view}
      onViewChange={setView}
      showToggle={serverManagement}
      servers={servers}
    />
  )
}
