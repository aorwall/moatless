import type { Task } from '@moatless/types'
import { Globe, Lock, Pencil } from 'lucide-react'
import { useState } from 'react'
import { useServers } from '@/features/preview/hooks/use-servers'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { MobileViewToggle } from '@/features/tasks/components/mobile-view-toggle'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { useUpdateTaskMetadata, useUpdateTaskVisibility } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { CreateRepoButton } from './create-repo-button'
import { EditTaskDialog } from './edit-task-dialog'

export function TaskLeftHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: repositories } = useRepositoriesList()
  const { data: servers } = useServers(taskId)
  const { view, setView, isMobile } = useTaskView()
  const updateVisibility = useUpdateTaskVisibility(taskId)
  const updateMetadata = useUpdateTaskMetadata(taskId, {
    onSuccess: () => setIsEditDialogOpen(false),
  })
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const {
    status,
    repositoryId,
    iterations,
    name,
    description,
    templateId,
    visibility,
    createdByName,
    createdBy,
  } = task

  // Get repository name or template ID for vibe tasks
  const repository = repositories?.find((repo) => repo.id === repositoryId)
  const displayName = repository?.name ?? ''

  // Get task title (use name if available, otherwise first line of initial message or "Untitled")
  const taskTitle = name || iterations?.[0]?.initialMessage?.split('\n')[0] || 'Untitled'

  // Show create repo button for Vibe tasks (tasks created from templates)
  const showCreateRepoButton = templateId != null

  const taskStatus: Pick<Task, 'status' | 'iterations'> = {
    status,
    iterations,
  }

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
          <span className="text-muted-foreground hidden sm:inline">•</span>
          <TaskStatusIcon task={taskStatus} showText={false} />
          {(createdByName || createdBy) && (
            <>
              <span className="text-muted-foreground hidden sm:inline">•</span>
              <span
                className="text-muted-foreground text-xs hidden sm:inline"
                title={createdByName || createdBy}
              >
                by {createdByName || createdBy}
              </span>
            </>
          )}
        </div>
      </div>

      <div className="flex items-center gap-2">
        {/* Visibility toggle */}
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          title={
            visibility === 'public'
              ? 'Public (click to make private)'
              : 'Private (click to make public)'
          }
          disabled={updateVisibility.isPending}
          onClick={() => updateVisibility.mutate(visibility === 'public' ? 'private' : 'public')}
        >
          {visibility === 'public' ? <Globe className="h-4 w-4" /> : <Lock className="h-4 w-4" />}
        </Button>

        {/* Create Repo button - only shown for Vibe tasks */}
        {showCreateRepoButton && (
          <CreateRepoButton
            repositoryId={repositoryId}
            taskName={taskTitle}
            repository={repository}
          />
        )}

        {/* Mobile view toggle - only shown on mobile */}
        {isMobile && (
          <MobileViewToggle currentView={view} onViewChange={setView} servers={servers} />
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
