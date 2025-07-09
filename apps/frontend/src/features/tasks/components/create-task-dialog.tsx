import type { UploadFileResponse } from '@moatless/shared/types/api'
import { useState } from 'react'
import { toast } from 'sonner'

import { ViewToggle } from '@/components/view-toggle'
import { ChatInput, type ChatInputMode } from '@/features/tasks/components/chat-input'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useCreateVibeTask } from '@/features/templates'
import { Dialog, DialogContent } from '@/lib/components/ui/dialog'
import { getErrorMessage } from '@/lib/utils/error'

interface CreateTaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function CreateTaskDialog({ open, onOpenChange }: CreateTaskDialogProps) {
  const [mode, setMode] = useState<ChatInputMode>(() => {
    // Load last mode from localStorage
    if (typeof window !== 'undefined') {
      return (localStorage.getItem('lastViewMode') as ChatInputMode) || 'code'
    }
    return 'code'
  })

  const createTask = useCreateTask({
    onSuccess: () => {
      // Close dialog on success
      onOpenChange(false)
    },
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  const createVibeTask = useCreateVibeTask({
    onSuccess: () => {
      // Close dialog on success
      onOpenChange(false)
    },
    onError: (error) => {
      toast.error('Failed to create vibe task', {
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
    clearMessage: () => void,
    model?: string,
    files?: UploadFileResponse[],
  ) => {
    // Extract file paths from uploaded files
    const filePaths = files?.map((f) => f.absolutePath) || []

    // Generate a repository name based on template and timestamp
    const repositoryName = `${templateId}-${Date.now()}`

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

  // Save mode to localStorage when it changes
  const handleModeChange = (newMode: ChatInputMode) => {
    setMode(newMode)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastViewMode', newMode)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl md:max-w-3xl lg:max-w-4xl">
        <div className="space-y-4">
          {/* View Toggle */}
          <div className="flex justify-center">
            <ViewToggle mode={mode} onModeChange={handleModeChange} />
          </div>
          
          {/* Chat Input */}
          <ChatInput
            showRepositorySelector={true}
            showTitle={false}
            mode={mode}
            onCreateTask={handleCreateTask}
            onCreateVibeTask={handleCreateVibeTask}
            isLoading={createTask.isPending || createVibeTask.isPending}
            className="w-full"
            minHeight="200px"
            maxHeight="500px"
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}