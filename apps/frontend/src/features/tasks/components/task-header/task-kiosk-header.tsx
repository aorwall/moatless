import { PanelRightClose, Pencil } from 'lucide-react'
import { useState } from 'react'
import { usePanelControls, useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useUpdateTaskMetadata } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { EditTaskDialog } from './edit-task-dialog'

export function TaskKioskHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const { toggleChat } = usePanelControls()
  const updateMetadata = useUpdateTaskMetadata(taskId, {
    onSuccess: () => setIsEditDialogOpen(false),
  })
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { name, description } = task
  const taskTitle = name || description?.split('\n')[0] || 'Untitled'

  return (
    <div className="absolute top-0 left-0 right-0 z-10 h-[58px] px-4 py-2 border-b flex items-center bg-background">
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
      </div>

      <div className="ml-auto">
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          title="Hide chat"
          onClick={toggleChat}
        >
          <PanelRightClose className="h-4 w-4" />
        </Button>
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
