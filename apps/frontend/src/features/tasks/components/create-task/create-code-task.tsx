import type { ClaudeModel, UploadFileResponse } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import {
  ChatInput,
  Files,
  Selectors,
  SendButton,
  UploadButton,
} from '@/lib/components/chat/chat-input'
import {
  BranchPopoverContent,
  PopoverSelector,
  SearchablePopoverContent,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '@/lib/components/chat/chat-input/selectors'
import { getErrorMessage } from '@/lib/utils/error'

interface CreateCodeTaskProps {
  className?: string
  maxHeight?: string
  minHeight?: string
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
  minHeight,
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
  const navigate = useNavigate()
  const [message, setMessage] = useState(defaultMessage || '')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [branch, setBranch] = useState('')

  // Use controlled selectors
  const modelSelector = useModelSelector(model, onModelChange)
  const agentModeSelector = useAgentModeSelector(agentMode, onAgentModeChange)
  const repoSelector = useRepositorySelector(repositoryId, onRepositoryChange, true)
  const branchSelector = useBranchSelector(repositoryId, branch, setBranch, true)

  // File upload hook
  const uploadFile = useUploadFile({
    onSuccess: (data) => {
      setUploadedFiles((prev) => [...prev, data])
    },
    onError: (error) => {
      console.error('File upload failed:', error)
      toast.error('Failed to upload file', {
        description: getErrorMessage(error),
      })
    },
  })

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

  const handleSubmit = () => {
    if (!repositoryId) {
      return
    }

    // Extract file paths from uploaded files
    const filePaths = uploadedFiles.map((f) => f.absolutePath)

    createTask.mutate(
      {
        agentType: 'claude-code',
        message: message.trim() || undefined,
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

  const isSendDisabled = !repositoryId
  console.log('isSendDisabled', isSendDisabled)

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
          className={
            maxHeight ? `max-h-[${maxHeight}]` : minHeight ? `min-h-[${minHeight}]` : undefined
          }
        >
          {uploadedFiles.length > 0 && (
            <Files
              files={uploadedFiles}
              onRemove={(id) => setUploadedFiles((prev) => prev.filter((f) => f.id !== id))}
            />
          )}

          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <PopoverSelector
                icon={repoSelector.icon}
                value={repoSelector.value}
                displayValue={repoSelector.displayValue}
                tooltip={repoSelector.tooltip}
                loading={repoSelector.loading}
                disabled={repoSelector.disabled}
              >
                <SearchablePopoverContent
                  items={repoSelector.items}
                  value={repoSelector.value}
                  onChange={repoSelector.onChange}
                  placeholder="Search repositories..."
                  emptyText="No repositories found"
                />
              </PopoverSelector>

              <PopoverSelector
                icon={branchSelector.icon}
                value={branchSelector.value}
                displayValue={branchSelector.displayValue}
                tooltip={branchSelector.tooltip}
                loading={branchSelector.loading}
                disabled={branchSelector.disabled}
              >
                <BranchPopoverContent
                  items={branchSelector.items}
                  value={branchSelector.value}
                  onChange={branchSelector.onChange}
                  currentBranch={branchSelector.currentBranch}
                  enableSearch={true}
                />
              </PopoverSelector>

              <PopoverSelector
                icon={modelSelector.icon}
                value={modelSelector.value}
                displayValue={modelSelector.displayValue}
                tooltip={modelSelector.tooltip}
                loading={modelSelector.loading}
                disabled={modelSelector.disabled}
              >
                <SimpleListPopoverContent
                  items={modelSelector.items}
                  value={modelSelector.value}
                  onChange={modelSelector.onChange}
                />
              </PopoverSelector>

              <PopoverSelector
                icon={agentModeSelector.icon}
                value={agentModeSelector.value}
                displayValue={agentModeSelector.displayValue}
                tooltip={agentModeSelector.tooltip}
                loading={agentModeSelector.loading}
                disabled={agentModeSelector.disabled}
              >
                <SimpleListPopoverContent
                  items={agentModeSelector.items}
                  value={agentModeSelector.value}
                  onChange={agentModeSelector.onChange}
                />
              </PopoverSelector>
            </Selectors>

            <div className="flex items-center gap-2">
              <UploadButton
                onFileSelect={(file) => uploadFile.mutate(file)}
                disabled={uploadFile.isPending}
              />
              <SendButton
                onSubmit={handleSubmit}
                disabled={isSendDisabled}
                loading={createTask.isPending}
                buttonText={
                  createTask.isPending
                    ? message.trim()
                      ? 'Starting...'
                      : 'Creating...'
                    : message.trim()
                      ? 'Start Task'
                      : 'Create Task'
                }
                message={message}
              />
            </div>
          </div>
        </ChatInput>
      </div>
    </div>
  )
}
