import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskStatus } from '@/features/tasks/contexts/task-providers'

export function FileExplorerHeader() {
  const { status } = useTaskStatus()

  if (!status) {
    return <WorkspaceHeader />
  }

  return <WorkspaceHeader />
}
