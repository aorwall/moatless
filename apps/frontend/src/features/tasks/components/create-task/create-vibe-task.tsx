import type { ClaudeModel } from '@moatless/types'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { SettingsDialog, useFigmaConfig } from '@/features/settings'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useCreateVibeTask } from '@/features/templates'
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
  PopoverSelector,
  SimpleListPopoverContent,
  useModelSelector,
  useTemplateSelector,
} from '@/lib/components/chat/chat-input/selectors'
import { getErrorMessage } from '@/lib/utils/error'

interface CreateVibeTaskProps {
  className?: string
  maxHeight?: string
  minHeight?: string
  showTitle?: boolean
  title?: string
  defaultMessage?: string
  model: ClaudeModel
  onModelChange: (model: ClaudeModel) => void
  templateId: string
  onTemplateChange: (templateId: string) => void
  onSuccess?: () => void
}

export function CreateVibeTask({
  className = '',
  maxHeight = '300px',
  minHeight,
  showTitle = false,
  title = 'What should we build today?',
  defaultMessage,
  model,
  onModelChange,
  templateId,
  onTemplateChange,
  onSuccess,
}: CreateVibeTaskProps) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [message, setMessage] = useState(defaultMessage || '')
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
  const templateSelector = useTemplateSelector(templateId, onTemplateChange, true)

  const createVibeTask = useCreateVibeTask({
    onSuccess: (data) => {
      // Invalidate new task data (including messages) - lists already handled by hook
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(data.taskId) })
      navigate({ to: `/tasks/${data.taskId}` })
    },
    onError: (error) => {
      toast.error('Failed to create vibe task', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleSubmit = () => {
    // Use the derived value from templateSelector which auto-selects first template
    const effectiveTemplateId = templateSelector.value

    if (!(message.trim() && effectiveTemplateId)) {
      return
    }

    createVibeTask.mutate(
      {
        agentType: 'claude-code',
        templateId: effectiveTemplateId,
        message: message.trim(),
        model: modelSelector.value,
        fileIds: taskInputs.getFileIds(),
        contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
        start: true,
      },
      {
        onSuccess: () => {
          setMessage('')
          taskInputs.clearAll()
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

  const canSubmit = !createVibeTask.isPending && !!message.trim() && !!templateSelector.value

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
          placeholder="Describe what you want to build from this template..."
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
                icon={templateSelector.icon}
                value={templateSelector.value}
                displayValue={templateSelector.displayValue}
                tooltip={templateSelector.tooltip}
                loading={templateSelector.loading}
              >
                <SimpleListPopoverContent
                  items={templateSelector.items}
                  value={templateSelector.value}
                  onChange={templateSelector.onChange}
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
                loading={createVibeTask.isPending}
                buttonText={createVibeTask.isPending ? 'Starting...' : 'Start Task'}
              />
            </div>
          </div>
        </ChatInput>
      </div>

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
