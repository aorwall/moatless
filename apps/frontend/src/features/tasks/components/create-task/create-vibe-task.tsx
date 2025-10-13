import type { ClaudeModel, UploadFileResponse } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { toast } from 'sonner'
import { useCreateVibeTask } from '@/features/templates'
import { useImagePaste } from '@/features/uploads/hooks/use-image-paste'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import {
  ChatInput,
  FileAttachments,
  Selectors,
  SendButton,
  UploadButton,
} from '@/lib/components/chat/chat-input'
import {
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
  title = 'What should we vibe today?',
  defaultMessage,
  model,
  onModelChange,
  templateId,
  onTemplateChange,
  onSuccess,
}: CreateVibeTaskProps) {
  const navigate = useNavigate()
  const [message, setMessage] = useState(defaultMessage || '')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])

  // Use controlled selectors
  const modelSelector = useModelSelector(model, onModelChange)
  const templateSelector = useTemplateSelector(templateId, onTemplateChange, true)

  // File upload hook
  const uploadFile = useUploadFile({
    onSuccess: (data) => {
      setUploadedFiles((prev) => [...prev, data])
    },
    onError: (error) => {
      toast.error('Failed to upload file', {
        description: getErrorMessage(error),
      })
    },
  })

  // Handle image paste
  const handlePaste = useImagePaste(uploadFile)

  const createVibeTask = useCreateVibeTask({
    onSuccess: (data) => {
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

    // Extract file paths from uploaded files
    const filePaths = uploadedFiles.map((f) => f.absolutePath)

    createVibeTask.mutate(
      {
        agentType: 'claude-code',
        templateId: effectiveTemplateId,
        message: message.trim(),
        model: modelSelector.value,
        filePaths,
        start: true,
      },
      {
        onSuccess: () => {
          setMessage('')
          setUploadedFiles([])
          onSuccess?.()
        },
      },
    )
  }

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
          onPaste={handlePaste}
        >
          {uploadedFiles.length > 0 && (
            <FileAttachments
              files={uploadedFiles}
              onRemove={(id) => setUploadedFiles((prev) => prev.filter((f) => f.id !== id))}
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
              <UploadButton
                onFileSelect={(file) => uploadFile.mutate(file)}
                disabled={uploadFile.isPending}
              />
              <SendButton
                disabled={!canSubmit}
                loading={createVibeTask.isPending}
                buttonText={createVibeTask.isPending ? 'Starting...' : 'Start Task'}
              />
            </div>
          </div>
        </ChatInput>
      </div>
    </div>
  )
}
