import type { ClaudeModel } from '@moatless/types'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { RepositoriesConfigurationDialog, useRepositoriesList } from '@/features/repositories'
import { SettingsDialog, useFigmaConfig } from '@/features/settings'
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
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  // Check Figma configuration status
  const { data: figmaConfig } = useFigmaConfig()

  // Task inputs hook (file uploads, context items, Figma)
  const taskInputs = useTaskInputs({
    enableContextItems: true,
    enableFigma: true,
  })

  // Use controlled selectors
  const modelSelector = useModelSelector(model, onModelChange)
  const agentModeSelector = useAgentModeSelector(agentMode, onAgentModeChange)
  const repoSelector = useRepositorySelector(repositoryId, onRepositoryChange, true)
  const branchSelector = useBranchSelector(repositoryId, branch, setBranch, true)

  // Fetch repositories list to detect when new repositories are added
  const { data: repositories } = useRepositoriesList()

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
        model,
        fileIds: taskInputs.getFileIds(),
        contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
        originalBranch: branchSelector.value,
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

  // Handle Figma import click - check config and open appropriate dialog
  const handleFigmaImportClick = useCallback(() => {
    if (figmaConfig?.connected) {
      taskInputs.setFigmaDialogOpen(true)
    } else {
      setSettingsDialogOpen(true)
    }
  }, [figmaConfig?.connected, taskInputs])

  const canSubmit = !!repositoryId && (!!message.trim() || taskInputs.contextItems.length > 0)

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
          placeholder="Describe your coding task or leave empty to just run the app..."
          className={
            maxHeight ? `max-h-[${maxHeight}]` : minHeight ? `min-h-[${minHeight}]` : undefined
          }
          canSubmit={canSubmit}
          onPaste={taskInputs.handlePaste}
        >
          {taskInputs.contextItems.length > 0 && (
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
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
              <PopoverSelector
                icon={taskInputs.attachmentSelector.icon}
                value={taskInputs.attachmentSelector.value}
                displayValue={taskInputs.attachmentSelector.displayValue}
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
                buttonText={
                  createTask.isPending
                    ? message.trim()
                      ? 'Starting...'
                      : 'Creating...'
                    : message.trim()
                      ? 'Start Task'
                      : 'Create Task'
                }
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
        onImportSuccess={taskInputs.handleFigmaImportSuccess}
      />

      <SettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
        defaultSection="figma"
      />
    </div>
  )
}
