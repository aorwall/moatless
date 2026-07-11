import { PanelRightClose } from 'lucide-react'
import { usePanelControls, useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { Button } from '@/lib/components/ui/button'

export function TaskKioskHeader() {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { toggleChat } = usePanelControls()
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
    </div>
  )
}
