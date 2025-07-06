import type { UploadFileResponse } from '@moatless/shared/types/api'
import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'

import { ViewToggle } from '@/components/view-toggle'
import { ChatInput, type ChatInputMode } from '@/features/tasks/components/chat-input'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useCreateVibeTask } from '@/features/templates'
import { getErrorMessage } from '@/lib/utils/error'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const [mode, setMode] = useState<ChatInputMode>(() => {
    // Load last mode from localStorage
    if (typeof window !== 'undefined') {
      return (localStorage.getItem('lastViewMode') as ChatInputMode) || 'code'
    }
    return 'code'
  })

  const createTask = useCreateTask({
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  const createVibeTask = useCreateVibeTask({
    onError: (error) => {
      toast.error('Failed to create vibe task', {
        description: getErrorMessage(error),
      })
    },
  })

  // Save mode to localStorage when it changes
  const handleModeChange = (newMode: ChatInputMode) => {
    setMode(newMode)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastViewMode', newMode)
    }
  }


  const handleCreateTask = (
    message: string,
    repositoryId: string,
    clearMessage: () => void,
    model?: string,
    files?: UploadFileResponse[],
    originalBranch?: string,
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
        originalBranch,
      },
      {
        onSuccess: () => {
          clearMessage()
        },
      },
    )
  }

  const handleCreateVibeTask = (
    message: string,
    templateId: string,
    repositoryName: string,
    clearMessage: () => void,
    model?: string,
    files?: UploadFileResponse[],
  ) => {
    // Extract file paths from uploaded files
    const filePaths = files?.map((f) => f.absolutePath) || []

    createVibeTask.mutate(
      {
        agentType: 'claude-code',
        templateId,
        repositoryName,
        message,
        model,
        filePaths,
        start: true,
      },
      {
        onSuccess: () => {
          clearMessage()
        },
      },
    )
  }

  const getTitle = () => {
    if (mode === 'vibe') {
      return 'What should we vibe today?'
    }
    return 'What should we build today?'
  }

  return (
    <div className="flex h-full items-center justify-center">
      <div className="w-full max-w-3xl px-6">
        {/* View Toggle */}
        <div className="flex justify-center mb-8">
          <ViewToggle mode={mode} onModeChange={handleModeChange} />
        </div>

        {/* Chat Input */}
        <ChatInput
          showRepositorySelector={true}
          showTitle={true}
          title={getTitle()}
          mode={mode}
          onCreateTask={handleCreateTask}
          onCreateVibeTask={handleCreateVibeTask}
          isLoading={createTask.isPending || createVibeTask.isPending}
          className="w-full"
          maxHeight="300px"
        />
      </div>
    </div>
  )
}
