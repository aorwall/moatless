import type { Task } from '@moatless/types'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { MobileViewToggle } from '@/features/tasks/components/mobile-view-toggle'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'

export function TaskLeftHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: repositories } = useRepositoriesList()
  const { view, setView, isMobile } = useTaskView()
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { status, agentType, repositoryId, iterations, vibe, templateId, name } = task

  // Get repository name or template ID for vibe tasks
  const repository = repositories?.find((repo) => repo.id === repositoryId)
  const repositoryName = repository?.name || repositoryId
  const displayName = vibe && templateId ? templateId : repositoryName

  // Get task title (use name if available, otherwise first line of initial message or agent type)
  const taskTitle = name || iterations?.[0]?.initialMessage?.split('\n')[0] || agentType

  const taskStatus: Pick<Task, 'status' | 'iterations'> = {
    status,
    iterations,
  }

  return (
    <div className="sticky top-0 z-10 h-[58px] px-4 py-2 border-b flex items-center justify-between flex-shrink-0 bg-background">
      <div className="flex items-center gap-3 min-w-0">
        <SidebarTrigger />
        <div className="flex items-center gap-2 text-sm min-w-0">
          <span className="font-medium truncate max-w-[300px]" title={taskTitle}>
            {taskTitle}
          </span>
          <span className="text-muted-foreground hidden sm:inline">•</span>
          <span
            className="text-muted-foreground truncate max-w-[150px] hidden sm:inline"
            title={displayName}
          >
            {displayName}
          </span>
          <span className="text-muted-foreground hidden sm:inline">•</span>
          <TaskStatusIcon task={taskStatus} showText={false} />
        </div>
      </div>

      {/* Mobile view toggle - only shown on mobile */}
      {isMobile && <MobileViewToggle currentView={view} onViewChange={setView} />}
    </div>
  )
}
