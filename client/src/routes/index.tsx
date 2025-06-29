import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'

import { ChatInput } from '@/features/tasks/components/chat-input'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const navigate = useNavigate()

  const createTask = useCreateTask({
    onSuccess: (data) => {
      navigate({ to: `/tasks/${data.taskId}` })
    },
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleCreateTask = (
    message: string,
    environmentId: string,
    clearMessage: () => void,
    model?: string,
  ) => {
    createTask.mutate(
      {
        agentType: 'claude-code',
        message,
        environmentId,
        model,
      },
      {
        onSuccess: () => {
          clearMessage()
        },
      },
    )
  }

  return (
    <div className="flex h-full items-center justify-center">
      <ChatInput
        showEnvironmentSelector={true}
        showTitle={true}
        title="What should we build today?"
        onCreateTask={handleCreateTask}
        isLoading={createTask.isPending}
        className="w-full max-w-2xl px-6"
      />
    </div>
  )
}
