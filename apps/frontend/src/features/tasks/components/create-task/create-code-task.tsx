import type { ClaudeModel, MessageContext, UploadFileResponse } from '@moatless/types'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { useFigmaClipboardPaste } from '@/features/figma/hooks/use-figma-clipboard-paste'
import { RepositoriesConfigurationDialog, useRepositoriesList } from '@/features/repositories'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useImagePaste } from '@/features/uploads/hooks/use-image-paste'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
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
  useAttachmentSelector,
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
  const [contextItems, setContextItems] = useState<MessageContext[]>([])
  const [branch, setBranch] = useState('')
  const [repoDialogOpen, setRepoDialogOpen] = useState(false)
  const [figmaDialogOpen, setFigmaDialogOpen] = useState(false)

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

  const attachmentSelector = useAttachmentSelector(uploadFile.isPending)

  // Handler for successful Figma import
  const handleFigmaImportSuccess = useCallback(
    (data: {
      importId: string
      fileId: string
      fileName: string
      nodeName: string
      hasImage: boolean
    }) => {
      const figmaContext: MessageContext = {
        type: 'figma',
        data: {
          importId: data.importId,
          fileName: data.fileName,
          nodeName: data.nodeName,
          hasImage: data.hasImage,
        },
      }
      setContextItems((prev) => [...prev, figmaContext])
      toast.success('Figma node imported successfully')
    },
    [],
  )

  // Figma clipboard paste handler
  const handleFigmaClipboardPaste = useFigmaClipboardPaste({
    onSuccess: useCallback((result: import('@/features/figma/api/figma').FigmaImportResponse) => {
      const figmaContext: MessageContext = {
        type: 'figma',
        data: {
          importId: result.importId,
          fileName: result.fileName,
          nodeName: result.nodeName,
          hasImage: result.hasImage,
        },
      }
      setContextItems((prev) => [...prev, figmaContext])
      toast.success(`Figma design imported: ${result.nodeName}`, {
        description: `From clipboard • ${result.fileName}`,
      })
    }, []),
  })

  // Handler to remove context item
  const handleRemoveContext = useCallback((index: number) => {
    setContextItems((prev) => prev.filter((_, i) => i !== index))
  }, [])

  // Handle image paste
  const handleImagePaste = useImagePaste(uploadFile)

  // Combined paste handler: Try Figma clipboard first, then fall back to image
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      // Try to handle as Figma clipboard data first
      const figmaHandled = handleFigmaClipboardPaste(e)
      if (figmaHandled) {
        return
      }

      // Fall back to image paste if not Figma data
      handleImagePaste(e)
    },
    [handleFigmaClipboardPaste, handleImagePaste],
  )

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

    // Extract file IDs from uploaded files
    const fileIds = uploadedFiles.map((f) => f.fileId)

    createTask.mutate(
      {
        agentType: 'claude-code',
        message: message.trim() || undefined,
        repositoryId,
        model,
        fileIds,
        contexts: contextItems.length > 0 ? contextItems : undefined,
        originalBranch: branchSelector.value,
        agentMode,
      },
      {
        onSuccess: () => {
          setMessage('')
          setUploadedFiles([])
          setContextItems([])
          setBranch('')
          onSuccess?.()
        },
      },
    )
  }

  const canSubmit = !!repositoryId

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
          onPaste={handlePaste}
        >
          {contextItems.length > 0 && (
            <ContextItems contexts={contextItems} onRemove={handleRemoveContext} />
          )}

          {uploadedFiles.length > 0 && (
            <FileAttachments
              files={uploadedFiles}
              onRemove={(fileId) =>
                setUploadedFiles((prev) => prev.filter((f) => f.fileId !== fileId))
              }
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
                icon={attachmentSelector.icon}
                value={attachmentSelector.value}
                displayValue={attachmentSelector.displayValue}
                tooltip={attachmentSelector.tooltip}
                loading={attachmentSelector.loading}
                disabled={attachmentSelector.disabled}
              >
                <AttachmentPopoverContent
                  onFileSelect={(file) => uploadFile.mutate(file)}
                  onFigmaImport={() => setFigmaDialogOpen(true)}
                  disabled={uploadFile.isPending}
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
        open={figmaDialogOpen}
        onOpenChange={setFigmaDialogOpen}
        onImportSuccess={handleFigmaImportSuccess}
      />
    </div>
  )
}
