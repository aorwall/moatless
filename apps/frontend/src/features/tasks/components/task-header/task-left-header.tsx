import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { taskWorkspaceLabel } from '@/features/tasks/utils/task-workspace-label'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { CreateRepoButton } from './create-repo-button'
import { ScheduledJobsPill } from './scheduled-jobs-pill'

/**
 * Desktop chat-panel header: sits above the chat side of the desktop task
 * split. Shows the task title, workspace/creator metadata, scheduled jobs, and
 * the create-repo action. The mobile view switcher lives in `TaskMobileHeader`.
 */
export function TaskLeftHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: repositories } = useRepositoriesList()
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { repositoryId, name, description, templateId, createdByDisplayName, createdBy } = task

  // The header shows the workspace the task runs in; the repository it resolves
  // to stays on the control panel. `repositories` is only needed for the
  // create-repo action below.
  const repository = repositories?.find((repo) => repo.id === repositoryId)
  const displayName = taskWorkspaceLabel(task)

  // Get task title (use name if available, otherwise first line of description or "Untitled")
  const taskTitle = name || description?.split('\n')[0] || 'Untitled'

  // Show create repo button for Vibe tasks (tasks created from templates)
  const showCreateRepoButton = templateId != null

  return (
    <div className="absolute top-0 left-0 right-0 z-10 h-[58px] px-4 py-2 border-b flex items-center justify-between flex-shrink-0 bg-background">
      <div className="flex items-center gap-3 min-w-0">
        <SidebarTrigger />
        <div className="flex items-center gap-2 text-sm min-w-0">
          <span className="font-medium truncate max-w-[300px]" title={taskTitle}>
            {taskTitle}
          </span>
          {displayName && (
            <>
              <span className="text-muted-foreground hidden sm:inline">•</span>
              <span
                className="text-muted-foreground truncate max-w-[150px] hidden sm:inline"
                title={displayName}
              >
                {displayName}
              </span>
            </>
          )}
          {(createdByDisplayName || createdBy) && (
            <>
              <span className="text-muted-foreground hidden sm:inline">•</span>
              <span
                className="text-muted-foreground text-xs hidden sm:inline"
                title={(createdByDisplayName || createdBy) ?? undefined}
              >
                by {createdByDisplayName || createdBy}
              </span>
            </>
          )}
        </div>
      </div>

      <div className="flex items-center gap-2">
        <ScheduledJobsPill jobs={sandboxStatus?.scheduledJobs ?? []} />

        {/* Create Repo button - only shown for Vibe tasks */}
        {showCreateRepoButton && (
          <CreateRepoButton
            repositoryId={repositoryId}
            taskName={taskTitle}
            repository={repository}
          />
        )}
      </div>
    </div>
  )
}
