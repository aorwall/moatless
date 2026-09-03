import type { MessageContextInput, UploadResponse } from '@moatless/api-client'
import type React from 'react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useAttachmentSelector } from '@/features/tasks/components/chat/chat-input/selectors'
import { useImagePaste } from '@/features/uploads/hooks/use-image-paste'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import { getErrorMessage } from '@/lib/utils/error'

export interface UseTaskInputsOptions {
  /**
   * External context items state (for ChatContainer prop-based management)
   * When provided, the hook will use this instead of internal state
   */
  contextItems?: MessageContextInput[] | undefined

  /**
   * Callback when context items change (for ChatContainer prop-based management)
   */
  onContextItemsChange?: ((contexts: MessageContextInput[]) => void) | undefined
}

export interface UseTaskInputsReturn {
  // File upload state and mutations
  uploadedFiles: UploadResponse[]
  uploadFile: ReturnType<typeof useUploadFile>
  attachmentSelector: ReturnType<typeof useAttachmentSelector>
  handleRemoveFile: (fileId: string) => void

  // Context items state and handlers
  contextItems: MessageContextInput[]
  handleRemoveContext: (index: number) => void
  handleAddContext: (context: MessageContextInput) => void

  // Skills activated for the next message (names only)
  selectedSkills: string[]
  handleAddSkill: (name: string) => void
  handleRemoveSkill: (name: string) => void

  // Paste handling
  handlePaste: (e: React.ClipboardEvent<HTMLTextAreaElement>) => void

  // Utilities
  getFileIds: () => string[]
  clearAll: () => void
}

/**
 * Consolidated hook for managing task input state: file uploads and context items.
 *
 * Owns `uploadedFiles`, `contextItems`, and `selectedSkills` together, so `clearAll`
 * resets all three after a send. A component that tracks any of them separately
 * falls out of sync with the others on send or reset.
 *
 * @example
 * ```tsx
 * const taskInputs = useTaskInputs()
 *
 * // In your component JSX:
 * <ChatInput onPaste={taskInputs.handlePaste}>
 *   {taskInputs.contextItems.length > 0 && (
 *     <ContextItems
 *       contexts={taskInputs.contextItems}
 *       onRemove={taskInputs.handleRemoveContext}
 *     />
 *   )}
 *   {taskInputs.uploadedFiles.length > 0 && (
 *     <FileAttachments
 *       files={taskInputs.uploadedFiles}
 *       onRemove={taskInputs.handleRemoveFile}
 *     />
 *   )}
 * </ChatInput>
 * ```
 */
export function useTaskInputs(options: UseTaskInputsOptions = {}): UseTaskInputsReturn {
  const { contextItems: externalContextItems, onContextItemsChange } = options

  // Determine if we're using external context management (for ChatContainer)
  const useExternalContext =
    externalContextItems !== undefined && onContextItemsChange !== undefined

  // File upload state
  const [uploadedFiles, setUploadedFiles] = useState<UploadResponse[]>([])

  // Context items state (internal or external)
  const [internalContextItems, setInternalContextItems] = useState<MessageContextInput[]>([])
  const contextItems = useExternalContext ? externalContextItems : internalContextItems

  // File upload mutation
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

  // Attachment selector
  const attachmentSelector = useAttachmentSelector(uploadFile.isPending)

  // File removal handler
  const handleRemoveFile = (fileId: string) => {
    setUploadedFiles((prev) => prev.filter((f) => f.fileId !== fileId))
  }

  // Context item handlers
  const handleRemoveContext = (index: number) => {
    if (useExternalContext && onContextItemsChange) {
      const newContexts = contextItems.filter((_, i) => i !== index)
      onContextItemsChange(newContexts)
    } else {
      setInternalContextItems((prev) => prev.filter((_, i) => i !== index))
    }
  }

  const handleAddContext = (context: MessageContextInput) => {
    if (useExternalContext && onContextItemsChange) {
      onContextItemsChange([...contextItems, context])
    } else {
      setInternalContextItems((prev) => [...prev, context])
    }
  }

  // Skills picked from the "/" dropdown. Names only — the sandbox resolves each
  // one's SKILL.md and frontloads its body.
  const [selectedSkills, setSelectedSkills] = useState<string[]>([])

  const handleAddSkill = (name: string) => {
    setSelectedSkills((prev) => (prev.includes(name) ? prev : [...prev, name]))
  }

  const handleRemoveSkill = (name: string) => {
    setSelectedSkills((prev) => prev.filter((s) => s !== name))
  }

  // Paste handling: images only
  const handlePaste = useImagePaste(uploadFile)

  // Utility: Get file IDs for API calls
  const getFileIds = () => {
    return uploadedFiles.map((f) => f.fileId)
  }

  // Utility: Clear all state
  const clearAll = () => {
    setUploadedFiles([])
    setSelectedSkills([])
    if (!useExternalContext) {
      setInternalContextItems([])
    } else if (onContextItemsChange) {
      onContextItemsChange([])
    }
  }

  return {
    // File uploads
    uploadedFiles,
    uploadFile,
    attachmentSelector,
    handleRemoveFile,

    // Context items
    contextItems,
    handleRemoveContext,
    handleAddContext,

    // Skills
    selectedSkills,
    handleAddSkill,
    handleRemoveSkill,

    // Paste handling
    handlePaste,

    // Utilities
    getFileIds,
    clearAll,
  }
}
