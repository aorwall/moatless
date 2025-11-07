import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskStatus } from '@/features/tasks/contexts/task-providers'

interface FileExplorerHeaderProps {
  serverManagement?: boolean
}

export function FileExplorerHeader({ serverManagement }: FileExplorerHeaderProps) {
  const { status } = useTaskStatus()

  if (!status) {
    return <WorkspaceHeader showToggle={serverManagement} />
  }

  return <WorkspaceHeader showToggle={serverManagement} />
}
