import type { ClaudeModel, UploadFileResponse } from '@moatless/types'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input/chat-input'
import {
  GenericSelector,
  useAgentModeSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '../chat-input/selectors'

interface CreateCodeTaskProps {
  className?: string
  maxHeight?: string
  showTitle?: boolean
  title?: string
  defaultMessage?: string
  model: ClaudeModel
  onModelChange: (model: ClaudeModel) => void
  agentMode: 'edit' | 'plan'
  onAgentModeChange: (mode: 'edit' | 'plan') => void
  repositoryId: string
  onRepositoryChange: (repoId: string) => void
  onSuccess?: () => void
}

export function CreateCodeTask({
  className = '',
  maxHeight = '300px',
  showTitle = false,
  title = 'What should we build today?',
  defaultMessage,
  model,
  onModelChange,
  agentMode,
  onAgentModeChange,
  repositoryId,
  onRepositoryChange,
  onSuccess,
}: CreateCodeTaskProps) {
  const [message, setMessage] = useState(defaultMessage || '')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [branch, setBranch] = useState('')

  // Use controlled selectors
  const modelSelector = useModelSelector(model, onModelChange)
  const agentModeSelector = useAgentModeSelector(agentMode, onAgentModeChange)
  const repoSelector = useRepositorySelector(repositoryId, onRepositoryChange, true)
  const branchSelector = useBranchSelector(repositoryId, branch, setBranch, true)

  const createTask = useCreateTask({
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleSubmit = () => {
    if (!message.trim() || !repositoryId) {
      return
    }

    // Extract file paths from uploaded files
    const filePaths = uploadedFiles.map((f) => f.absolutePath)

    createTask.mutate(
      {
        agentType: 'claude-code',
        message: message.trim(),
        repositoryId,
        model,
        filePaths,
        originalBranch: branchSelector.value,
        agentMode,
      },
      {
        onSuccess: () => {
          setMessage('')
          setUploadedFiles([])
          setBranch('')
          onSuccess?.()
        },
      },
    )
  }

  const isSendDisabled = createTask.isPending || !message.trim() || !repositoryId

  return (
    <div className={className}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto' : ''}>
        <ChatInput
          message={message}
          onMessageChange={setMessage}
          onSubmit={handleSubmit}
          disabled={isSendDisabled}
          placeholder="Describe your coding task or feature... (use @ to reference files)"
          buttonText={createTask.isPending ? 'Starting...' : 'Start Task'}
          loading={createTask.isPending}
          maxHeight={maxHeight}
          repositoryId={repoSelector.value}
          enableFileUpload={true}
          files={uploadedFiles}
          onFilesChange={setUploadedFiles}
          selectors={
            <>
              <GenericSelector {...repoSelector} />
              <GenericSelector {...branchSelector} />
              <GenericSelector {...modelSelector} />
              <GenericSelector {...agentModeSelector} />
            </>
          }
        />
      </div>
    </div>
  )
}
