import { Globe, Lock, MoreVertical, Pencil, RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useWorkspaceRestart, useWorkspaceStatus } from '@/features/tasks'
import { MobileViewToggle } from '@/features/tasks/components/mobile-view-toggle'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { useUpdateTaskMetadata, useUpdateTaskVisibility } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'
import { SidebarTrigger } from '@/lib/components/ui/sidebar'
import { CreateRepoButton } from './create-repo-button'
import { EditTaskDialog } from './edit-task-dialog'

export function TaskLeftHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: repositories } = useRepositoriesList()
  const { data: workspaceStatus } = useWorkspaceStatus(taskId, taskQuery.data)
  const servers = workspaceStatus?.servers
  const { view, setView, isMobile } = useTaskView()
  const updateVisibility = useUpdateTaskVisibility(taskId)
  const updateMetadata = useUpdateTaskMetadata(taskId, {
    onSuccess: () => setIsEditDialogOpen(false),
  })
  const workspaceRestart = useWorkspaceRestart(taskId)
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const [isMenuOpen, setIsMenuOpen] = useState(false)
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const {
    status,
    repositoryId,
    name,
    description,
    templateId,
    visibility,
    createdByDisplayName,
    createdBy,
  } = task

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
          <span className="text-muted-foreground hidden sm:inline">•</span>
          <TaskStatusIcon task={{ status, totalCostUsd: task.totalCostUsd }} showText={false} />
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
        {/* Task actions menu */}
        <Popover open={isMenuOpen} onOpenChange={setIsMenuOpen}>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon" className="h-7 w-7" title="Task actions">
              <MoreVertical className="h-4 w-4" />
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-48 p-1" align="end">
            <Button
              variant="ghost"
              className="w-full justify-start gap-2 h-8 px-2"
              disabled={updateVisibility.isPending}
              onClick={() => {
                updateVisibility.mutate(visibility === 'public' ? 'private' : 'public')
                setIsMenuOpen(false)
              }}
            >
              {visibility === 'public' ? (
                <>
                  <Lock className="h-4 w-4" />
                  Make Private
                </>
              ) : (
                <>
                  <Globe className="h-4 w-4" />
                  Make Public
                </>
              )}
            </Button>
            <Button
              variant="ghost"
              className="w-full justify-start gap-2 h-8 px-2"
              disabled={workspaceRestart.isPending}
              onClick={() => {
                workspaceRestart.mutate()
                setIsMenuOpen(false)
              }}
            >
              <RefreshCw
                className={`h-4 w-4 ${workspaceRestart.isPending ? 'animate-spin' : ''}`}
              />
              Restart Workspace
            </Button>
          </PopoverContent>
        </Popover>

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
