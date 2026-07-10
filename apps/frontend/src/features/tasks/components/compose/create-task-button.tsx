import { Plus } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { useComposeTask } from '../../contexts/compose-task-provider'

interface CreateTaskButtonProps {
  className?: string
}

/** Opens a new Gmail-style compose window. Placed next to the app logo. */
export function CreateTaskButton({ className }: CreateTaskButtonProps) {
  const { open } = useComposeTask()

  return (
    <Button
      variant="outline"
      size="sm"
      className={cn('h-8 gap-1.5', className)}
      onClick={open}
      title="Compose a new task"
    >
      <Plus className="h-4 w-4" />
      New task
    </Button>
  )
}
