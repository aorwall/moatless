import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import {
  RepositoriesConfigurationDialog,
  useRepositoriesList,
  useRepository,
} from '@/features/repositories'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import {
  ChatInput,
  ContextItems,
  FileAttachments,
  Selectors,
  SendButton,
} from '@/lib/components/chat/chat-input'
import {
  AttachmentPopoverContent,
  BranchPopoverContent,
  PopoverSelector,
  SearchablePopoverContent,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '@/lib/components/chat/chat-input/selectors'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
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
  const queryClient = useQueryClient()
  const [message, setMessage] = useState(defaultMessage || '')
  const [branch, setBranch] = useState('')
  const [repoDialogOpen, setRepoDialogOpen] = useState(false)

  // Check if model selector should be shown
  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  // Task inputs hook (file uploads, context items, Figma)
  const taskInputs = useTaskInputs({
    enableContextItems: true,
    enableFigma: true,
  })

  // Fetch repositories list to detect when new repositories are added
  const { data: repositories } = useRepositoriesList()

  // Fetch full repository details to check worktree/clone settings
  const { data: repositoryDetails, error: repositoryError } = useRepository(repositoryId, {
    enabled: !!repositoryId,
  })

  const isTemplate = repositoryDetails?.kind === 'template'

  // Clear stored repository ID on 404 error
  useEffect(() => {
    if (
      repositoryError &&
      'status' in repositoryError &&
      (repositoryError as { status: number }).status === 404
    ) {
      onRepositoryChange('')
      toast.info('Previously selected repository no longer exists')
    }
  }, [repositoryError, onRepositoryChange])

  // Use controlled selectors
  const modelSelector = useModelSelector(
    isModelSelectorEnabled ? model : DEFAULT_CLAUDE_MODEL,
    isModelSelectorEnabled
      ? onModelChange
      : () => {
          /* no-op: model selector disabled */
        },
  )
  const agentModeSelector = useAgentModeSelector(agentMode, onAgentModeChange)
  const repoSelector = useRepositorySelector(repositoryId, onRepositoryChange, true)
  const branchSelector = useBranchSelector(repositoryId, branch, setBranch, true, repositoryDetails)

  // Auto-select first repository when dialog closes and a repository is available but none selected
  useEffect(() => {
    if (!(repoDialogOpen || repositoryId) && repositories && repositories.length > 0) {
      onRepositoryChange(repositories[0].id)
    }
  }, [repoDialogOpen, repositoryId, repositories, onRepositoryChange])

  const createTask = useCreateTask({
    onSuccess: (data) => {
      // Invalidate new task data (including messages) - lists already handled by hook
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(data.taskId) })
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

    createTask.mutate(
      {
        agentType: 'claude-code',
        message: message.trim() || undefined,
        repositoryId,
        model: isModelSelectorEnabled ? model : DEFAULT_CLAUDE_MODEL,
        fileIds: taskInputs.getFileIds(),
        contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
        branch: branchSelector.value,
        agentMode,
      },
      {
        onSuccess: () => {
          setMessage('')
          taskInputs.clearAll()
          setBranch('')
          onSuccess?.()
        },
      },
    )
  }

  // Handle Figma import click - FigmaImportDialog handles unconfigured case internally
  const handleFigmaImportClick = () => {
    taskInputs.setFigmaDialogOpen(true)
  }

  const canSubmit = !!repositoryId

  const placeholder = isTemplate
    ? 'Describe what you want to build...'
    : 'Describe your coding task or leave empty to just run the app...'

  const chatInputClassName = maxHeight
    ? `max-h-[${maxHeight}]`
    : minHeight
      ? `min-h-[${minHeight}]`
      : undefined

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
          placeholder={placeholder}
          className={chatInputClassName}
          canSubmit={canSubmit}
          onPaste={taskInputs.handlePaste}
        >
          {(taskInputs.contextItems.length > 0 || taskInputs.figmaImportPending) && (
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
              isLoading={taskInputs.figmaImportPending}
            />
          )}

          {taskInputs.uploadedFiles.length > 0 && (
            <FileAttachments
              files={taskInputs.uploadedFiles}
              onRemove={taskInputs.handleRemoveFile}
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
                  onEmptyAction={() => setRepoDialogOpen(true)}
                  emptyActionText="Create Repository"
                />
              </PopoverSelector>

              {!(branchSelector.hidden || isTemplate) && (
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
              )}

              {isModelSelectorEnabled && (
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
              )}

              {!isTemplate && (
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
              )}
            </Selectors>

            <div className="flex items-center gap-2">
              <PopoverSelector
                icon={taskInputs.attachmentSelector.icon}
                value={taskInputs.attachmentSelector.value}
                tooltip={taskInputs.attachmentSelector.tooltip}
                loading={taskInputs.attachmentSelector.loading}
                disabled={taskInputs.attachmentSelector.disabled}
              >
                <AttachmentPopoverContent
                  onFileSelect={(file) => taskInputs.uploadFile.mutate(file)}
                  onFigmaImport={handleFigmaImportClick}
                  disabled={taskInputs.uploadFile.isPending}
                />
              </PopoverSelector>
              <SendButton
                disabled={!canSubmit}
                loading={createTask.isPending}
                buttonText={getSubmitButtonText(createTask.isPending, message)}
              />
            </div>
          </div>
        </ChatInput>
      </div>

      <RepositoriesConfigurationDialog
        open={repoDialogOpen}
        onOpenChange={setRepoDialogOpen}
        defaultToCreate={true}
      />

      <FigmaImportDialog
        open={taskInputs.figmaDialogOpen}
        onOpenChange={taskInputs.setFigmaDialogOpen}
        importMutation={taskInputs.figmaUrlImport}
      />
    </div>
  )
}

function getSubmitButtonText(isPending: boolean, message: string): string {
  if (isPending) {
    return message.trim() ? 'Starting...' : 'Creating...'
  }
  return message.trim() ? 'Start Task' : 'Create Task'
}
