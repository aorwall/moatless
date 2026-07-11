import { Pencil } from 'lucide-react'
import { useState } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { ViewSwitcher } from '@/features/tasks/components/view-switcher'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { type TaskView, useTaskView } from '@/features/tasks/hooks/use-task-view'
import { useUpdateTaskMetadata } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { CreateRepoButton } from './create-repo-button'
import { EditTaskDialog } from './edit-task-dialog'
import { ScheduledJobsPill } from './scheduled-jobs-pill'

interface TaskLeftHeaderProps {
  /**
   * Render the mobile view toggle. Owned by the layout: the mobile layout sets
   * this, the desktop layout leaves it off (tabs live in the workspace header).
   */
  showViewToggle?: boolean
  /** Default view when the URL has no `?view=` — must match the layout's default. */
  defaultView?: TaskView
}

export function TaskLeftHeader({ showViewToggle = false, defaultView }: TaskLeftHeaderProps = {}) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: repositories } = useRepositoriesList()
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const servers = sandboxStatus?.servers
  const { view, setView } = useTaskView({ fallback: defaultView })
  const updateMetadata = useUpdateTaskMetadata(taskId, {
    onSuccess: () => setIsEditDialogOpen(false),
  })
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { repositoryId, name, description, templateId, createdByDisplayName, createdBy } = task

  // Get repository name or template ID for vibe tasks
  const repository = repositories?.find((repo) => repo.id === repositoryId)
  const displayName = repository?.name ?? ''

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
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0"
            title="Edit task name and description"
            onClick={() => setIsEditDialogOpen(true)}
          >
            <Pencil className="h-3 w-3" />
          </Button>
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

        {/* Mobile view switcher - shown when the layout requests it (mobile) */}
        {showViewToggle && (
          <ViewSwitcher currentView={view} onViewChange={setView} servers={servers} />
        )}
      </div>

      <EditTaskDialog
        isOpen={isEditDialogOpen}
        onOpenChange={setIsEditDialogOpen}
        currentName={name || ''}
        currentDescription={description || ''}
        onSubmit={(updates) => updateMetadata.mutate(updates)}
        isPending={updateMetadata.isPending}
      />
    </div>
  )
}
