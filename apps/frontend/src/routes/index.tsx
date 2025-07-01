import type { UploadFileResponse } from '@moatless/shared/types/api'
import { createFileRoute } from '@tanstack/react-router'
import { toast } from 'sonner'

import { ChatInput } from '@/features/tasks/components/chat-input'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const createTask = useCreateTask({
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })


  const handleCreateTask = (
    message: string,
    repositoryId: string,
    clearMessage: () => void,
    model?: string,
    files?: UploadFileResponse[],
  ) => {
    // Extract file paths from uploaded files
    const filePaths = files?.map((f) => f.absolutePath) || []

    createTask.mutate(
      {
        agentType: 'claude-code',
        message,
        repositoryId,
        model,
        filePaths,
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
        showRepositorySelector={true}
        showTitle={true}
        title="What should we build today?"
        onCreateTask={handleCreateTask}
        isLoading={createTask.isPending}
        className="w-full max-w-3xl px-6"
        maxHeight="300px"
      />
    </div>
  )
}
