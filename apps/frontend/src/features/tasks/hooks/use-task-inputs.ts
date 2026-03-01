import type { MessageContextInput, UploadResponse } from '@moatless/api-client'
import type React from 'react'
import { useCallback, useState } from 'react'
import { toast } from 'sonner'
import type { FigmaImportResponse } from '@/features/figma/api/figma'
import { useFigmaClipboardPaste } from '@/features/figma/hooks/use-figma-clipboard-paste'
import { useFigmaImport } from '@/features/figma/hooks/use-figma-import'
import { useImagePaste } from '@/features/uploads/hooks/use-image-paste'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import { useAttachmentSelector } from '@/lib/components/chat/chat-input/selectors'
import { getErrorMessage } from '@/lib/utils/error'

export interface UseTaskInputsOptions {
  /**
   * Enable context items management. Default: true
   */
  enableContextItems?: boolean

  /**
   * Enable Figma integration (clipboard paste + import dialog). Default: true
   */
  enableFigma?: boolean

  /**
   * External context items state (for ChatContainer prop-based management)
   * When provided, the hook will use this instead of internal state
   */
  contextItems?: MessageContextInput[]

  /**
   * Callback when context items change (for ChatContainer prop-based management)
   */
  onContextItemsChange?: (contexts: MessageContextInput[]) => void
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

  // Figma integration
  figmaDialogOpen: boolean
  setFigmaDialogOpen: (open: boolean) => void
  figmaUrlImport: ReturnType<typeof useFigmaImport>
  figmaImportPending: boolean
  handleFigmaImportSuccess: (data: {
    importId: string
    fileId: string
    fileName: string
    nodeName: string
    hasImage: boolean
  }) => void

  // Paste handling
  handlePaste: (e: React.ClipboardEvent<HTMLTextAreaElement>) => void

  // Utilities
  getFileIds: () => string[]
  clearAll: () => void
}

/**
 * Consolidated hook for managing task input state: file uploads, context items, and Figma integration.
 *
 * This hook eliminates duplication across task creation components by providing a single
 * source of truth for all file/context management logic.
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
 *
 * <FigmaImportDialog
 *   open={taskInputs.figmaDialogOpen}
 *   onOpenChange={taskInputs.setFigmaDialogOpen}
 *   onImportSuccess={taskInputs.handleFigmaImportSuccess}
 * />
 * ```
 */
export function useTaskInputs(options: UseTaskInputsOptions = {}): UseTaskInputsReturn {
  const {
    enableContextItems = true,
    enableFigma = true,
    contextItems: externalContextItems,
    onContextItemsChange,
  } = options

  // Determine if we're using external context management (for ChatContainer)
  const useExternalContext =
    externalContextItems !== undefined && onContextItemsChange !== undefined

  // File upload state
  const [uploadedFiles, setUploadedFiles] = useState<UploadResponse[]>([])

  // Context items state (internal or external)
  const [internalContextItems, setInternalContextItems] = useState<MessageContextInput[]>([])
  const contextItems = useExternalContext ? externalContextItems : internalContextItems

  // Figma dialog state
  const [figmaDialogOpen, setFigmaDialogOpen] = useState(false)

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
  const handleRemoveFile = useCallback((fileId: string) => {
    setUploadedFiles((prev) => prev.filter((f) => f.fileId !== fileId))
  }, [])

  // Context item handlers
  const handleRemoveContext = useCallback(
    (index: number) => {
      if (useExternalContext && onContextItemsChange) {
        const newContexts = contextItems.filter((_, i) => i !== index)
        onContextItemsChange(newContexts)
      } else {
        setInternalContextItems((prev) => prev.filter((_, i) => i !== index))
      }
    },
    [useExternalContext, onContextItemsChange, contextItems],
  )

  const handleAddContext = useCallback(
    (context: MessageContextInput) => {
      if (useExternalContext && onContextItemsChange) {
        onContextItemsChange([...contextItems, context])
      } else {
        setInternalContextItems((prev) => [...prev, context])
      }
    },
    [useExternalContext, onContextItemsChange, contextItems],
  )

  // Figma import success handler
  const handleFigmaImportSuccess = useCallback(
    (data: {
      importId: string
      fileId: string
      fileName: string
      nodeName: string
      hasImage: boolean
    }) => {
      if (!enableContextItems) {
        return
      }

      const figmaContext: MessageContextInput = {
        type: 'figma',
        referenceId: data.importId,
        data: {
          importId: data.importId,
          fileName: data.fileName,
          nodeName: data.nodeName,
          hasImage: data.hasImage,
        },
      }
      handleAddContext(figmaContext)
      toast.success('Figma node imported successfully')
    },
    [enableContextItems, handleAddContext],
  )

  // Figma URL import mutation (for FigmaImportDialog)
  const figmaUrlImport = useFigmaImport({
    onSuccess: (data) => {
      handleFigmaImportSuccess({
        importId: data.importId,
        fileId: data.fileId ?? '',
        fileName: data.fileName ?? '',
        nodeName: data.nodeName ?? '',
        hasImage: data.hasImage,
      })
      setFigmaDialogOpen(false)
    },
    onError: (error) => {
      toast.error('Failed to import from Figma', {
        description: getErrorMessage(error),
      })
    },
  })

  // Figma clipboard paste handler
  const figmaClipboardPaste = useFigmaClipboardPaste({
    onSuccess: useCallback(
      (result: FigmaImportResponse) => {
        if (!enableContextItems) {
          return
        }

        const figmaContext: MessageContextInput = {
          type: 'figma',
          referenceId: result.importId,
          data: {
            importId: result.importId,
            fileName: result.fileName ?? '',
            nodeName: result.nodeName ?? '',
            hasImage: result.hasImage,
          },
        }
        handleAddContext(figmaContext)
        toast.success(`Figma design imported: ${result.nodeName}`, {
          description: `From clipboard • ${result.fileName}`,
        })
      },
      [enableContextItems, handleAddContext],
    ),
  })

  // Image paste handler
  const handleImagePaste = useImagePaste(uploadFile)

  // Combined paste handler: Figma first, then image
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      if (enableFigma) {
        // Try to handle as Figma clipboard data first
        const figmaHandled = figmaClipboardPaste.handlePaste(e)
        if (figmaHandled) {
          return
        }
      }

      // Fall back to image paste if not Figma data
      handleImagePaste(e)
    },
    [enableFigma, figmaClipboardPaste, handleImagePaste],
  )

  // Utility: Get file IDs for API calls
  const getFileIds = useCallback(() => {
    return uploadedFiles.map((f) => f.fileId)
  }, [uploadedFiles])

  // Utility: Clear all state
  const clearAll = useCallback(() => {
    setUploadedFiles([])
    if (!useExternalContext) {
      setInternalContextItems([])
    } else if (onContextItemsChange) {
      onContextItemsChange([])
    }
  }, [useExternalContext, onContextItemsChange])

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

    // Figma integration
    figmaDialogOpen,
    setFigmaDialogOpen,
    figmaUrlImport,
    figmaImportPending: figmaUrlImport.isPending || figmaClipboardPaste.isPending,
    handleFigmaImportSuccess,

    // Paste handling
    handlePaste,

    // Utilities
    getFileIds,
    clearAll,
  }
}
