import type { UploadFileResponse } from '@moatless/types'
import { ArrowUp, Circle, Loader2, Paperclip, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { FileAutocomplete } from '@/features/tasks/components/file-autocomplete.tsx'
import { useUploadFile } from '@/features/uploads/hooks/use-upload.ts'
import { Button } from '@/lib/components/ui/button.tsx'
import { Textarea } from '@/lib/components/ui/textarea.tsx'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip.tsx'
import { useUniversalFileAutocomplete } from '@/lib/hooks/use-universal-file-autocomplete.ts'
import { getErrorMessage } from '@/lib/utils/error.ts'

interface ChatFormProps {
  // Core functionality
  message: string
  onMessageChange: (message: string) => void
  onSubmit: () => void

  // UI configuration
  placeholder?: string
  buttonText?: string
  disabled?: boolean
  loading?: boolean
  running?: boolean
  maxHeight?: string

  // Context
  taskId?: string
  repositoryId?: string
  isTaskContext?: boolean

  // Composable selectors
  selectors?: React.ReactNode

  // File upload
  files?: UploadFileResponse[]
  onFilesChange?: (files: UploadFileResponse[]) => void
}

export function ChatForm({
  message,
  onMessageChange,
  onSubmit,
  placeholder = 'Type a message...',
  buttonText = 'Send',
  disabled = false,
  loading = false,
  running = false,
  maxHeight = '200px',
  taskId: _taskId,
  repositoryId,
  isTaskContext = false,
  selectors,
  files = [],
  onFilesChange,
}: ChatFormProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const formRef = useRef<HTMLFormElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [dropdownPosition, setDropdownPosition] = useState<'above' | 'below'>('below')

  // File autocomplete functionality
  const {
    inputRef,
    isOpen: isAutocompleteOpen,
    results: autocompleteResults,
    selectedIndex,
    isLoading: isAutocompleteLoading,
    error: autocompleteError,
    handleInputChange,
    handleKeyDown: handleAutocompleteKeyDown,
    selectFile,
  } = useUniversalFileAutocomplete({
    repositoryId,
    value: message,
    onValueChange: onMessageChange,
  })

  // File upload functionality
  const uploadFile = useUploadFile({
    onSuccess: (data) => {
      if (onFilesChange) {
        onFilesChange([...files, data])
      }
    },
    onError: (error) => {
      console.error('File upload failed:', error)
      toast.error('Failed to upload file', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleFileUpload = () => {
    fileInputRef.current?.click()
  }

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) {
      // Always upload immediately to the general uploads endpoint
      uploadFile.mutate(file, {
        onSuccess: (data) => {
          if (onFilesChange) {
            onFilesChange([...files, data])
          }
        },
      })
    }
    // Reset input so same file can be selected again
    e.target.value = ''
  }

  const removeFile = (id: string) => {
    if (onFilesChange) {
      onFilesChange(files.filter((f) => f.id !== id))
    }
  }

  // Callback ref to handle initial resize and ref assignment
  const textareaCallbackRef = (node: HTMLTextAreaElement | null) => {
    // Assign to the autocomplete ref
    inputRef.current = node

    if (node) {
      // Set initial height using callback ref pattern
      node.style.height = 'auto'
      const scrollHeight = node.scrollHeight
      const maxHeightPx = Number.parseInt(maxHeight)
      const newHeight = Math.min(scrollHeight, maxHeightPx)
      node.style.height = `${newHeight}px`
    }
  }

  // Derive dropdown position when autocomplete opens
  const calculateDropdownPosition = useCallback(() => {
    if (isAutocompleteOpen && containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect()
      const spaceBelow = window.innerHeight - rect.bottom
      const dropdownHeight = 240 // max-h-60 = 15rem = 240px

      // If not enough space below and we're in task context (likely at bottom), show above
      const newPosition = spaceBelow < dropdownHeight && isTaskContext ? 'above' : 'below'
      if (newPosition !== dropdownPosition) {
        setDropdownPosition(newPosition)
      }
    }
  }, [isAutocompleteOpen, isTaskContext, dropdownPosition])

  // Calculate position when autocomplete opens (effect needed for timing)
  useEffect(() => {
    if (isAutocompleteOpen) {
      calculateDropdownPosition()
    }
  }, [isAutocompleteOpen, calculateDropdownPosition])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Let autocomplete handle key events first
    if (isAutocompleteOpen) {
      handleAutocompleteKeyDown(e)
      if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'].includes(e.key)) {
        return
      }
    }

    if (e.key === 'Enter' && !e.shiftKey && !isAutocompleteOpen && !disabled) {
      e.preventDefault()
      onSubmit()
    }
  }

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value
    const cursorPosition = e.target.selectionStart || 0
    onMessageChange(value)
    handleInputChange(value, cursorPosition)

    // Auto-resize textarea on input
    const textarea = e.target
    textarea.style.height = 'auto'
    const scrollHeight = textarea.scrollHeight
    const maxHeightPx = Number.parseInt(maxHeight)
    const newHeight = Math.min(scrollHeight, maxHeightPx)
    textarea.style.height = `${newHeight}px`
  }

  const handlePaste = async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items
    if (!items) return

    // Check for image in clipboard first
    let hasImage = false
    for (let i = 0; i < items.length; i++) {
      const item = items[i]

      if (item.type.indexOf('image') !== -1) {
        hasImage = true
        e.preventDefault() // Prevent default paste behavior for images

        const blob = item.getAsFile()
        if (blob) {
          // Generate a filename based on timestamp
          const timestamp = Date.now()
          const extension = blob.type.split('/')[1] || 'png'
          const filename = `pasted-image-${timestamp}.${extension}`

          // Create a File object from the blob
          const file = new File([blob], filename, { type: blob.type })

          // Upload the file using existing upload functionality
          uploadFile.mutate(file, {
            onSuccess: (data) => {
              if (onFilesChange) {
                onFilesChange([...files, data])
              }
            },
          })
        }
        break // Only handle the first image
      }
    }

    // If no image was found, allow normal text paste behavior
    if (!hasImage) {
      // Let the browser handle normal text paste - don't prevent default
      return
    }
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!disabled) {
      onSubmit()
    }
  }

  return (
    <form
      className="group flex flex-col gap-2 rounded-3xl border border-border bg-card p-3 relative"
      onSubmit={handleSubmit}
      ref={formRef}
    >
      <div ref={containerRef}>
        <div className="relative flex flex-1 items-center">
          <Textarea
            ref={textareaCallbackRef}
            placeholder={placeholder}
            value={message}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            className="flex w-full ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none text-[16px] leading-snug placeholder-shown:text-ellipsis placeholder-shown:whitespace-nowrap md:text-base focus-visible:ring-0 focus-visible:ring-offset-0 scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted bg-transparent focus:bg-transparent flex-1 m-1 rounded-md p-0 border-0 shadow-none overflow-y-auto"
            style={{ minHeight: '40px', maxHeight }}
            disabled={loading}
          />
          {isAutocompleteOpen && (
            <FileAutocomplete
              results={autocompleteResults}
              selectedIndex={selectedIndex}
              onSelect={selectFile}
              isLoading={isAutocompleteLoading}
              error={autocompleteError}
              position={dropdownPosition}
            />
          )}
        </div>
      </div>
      {/* Uploaded files display */}
      {files.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {files.map((file) => {
            const isImage = file.mimeType.startsWith('image/')
            // Use the upload endpoint to get the image
            const imageUrl = isImage ? `/api/v1/uploads/${file.id}/${file.filename}` : null

            return (
              <div
                key={file.id}
                className="flex items-center gap-1 px-2 py-1 bg-secondary/10 rounded text-xs"
              >
                {isImage && imageUrl && (
                  <img
                    src={imageUrl}
                    alt={file.filename}
                    className="h-8 w-8 object-cover rounded"
                  />
                )}
                <span className="max-w-[150px] truncate">{file.filename}</span>
                <button
                  type="button"
                  onClick={() => removeFile(file.id)}
                  className="text-muted-foreground hover:text-foreground transition-colors"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            )
          })}
        </div>
      )}
      <div className="flex items-center justify-between gap-2">
        {/* Left side - Composable selectors */}
        <div className="flex items-center gap-2">{selectors}</div>

        {/* Right side - Upload and Send buttons */}
        <div className="flex items-center gap-2">
          {/* Upload button - always available */}
          <input
            ref={fileInputRef}
            type="file"
            onChange={handleFileChange}
            className="hidden"
            accept="image/*,.pdf,.txt,.md,.js,.ts,.jsx,.tsx,.py,.java,.c,.cpp,.h,.hpp,.cs,.rb,.go,.rs,.swift,.kt,.m,.mm,.php,.html,.css,.scss,.json,.xml,.yaml,.yml,.toml,.ini,.conf,.sh,.bash,.zsh,.fish,.ps1,.bat,.cmd"
          />
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="h-6 w-6"
                  onClick={handleFileUpload}
                  disabled={uploadFile.isPending}
                >
                  <Paperclip className="h-4 w-4" />
                  <span className="sr-only">Upload file</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                <p>Attach files to your message</p>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
          {running ? (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="submit"
                    disabled={disabled}
                    size="sm"
                    className="flex items-center justify-center gap-1.5 rounded-full bg-transparent border border-border text-muted-foreground transition-all duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 px-3 text-xs font-medium hover:bg-accent hover:text-accent-foreground hover:border-accent"
                  >
                    <span>{buttonText}</span>
                    <Circle className="h-3 w-3 text-muted-foreground" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  <p>Click to stop the agent</p>
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          ) : (
            <Button
              type="submit"
              disabled={disabled}
              size="icon"
              className="flex size-6 items-center justify-center rounded-full bg-foreground text-background transition-opacity duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 w-6"
            >
              {loading ? (
                <Loader2 className="h-5 w-5 animate-spin" />
              ) : (
                <ArrowUp className="h-5 w-5" />
              )}
              <span className="sr-only">{buttonText}</span>
            </Button>
          )}
        </div>
      </div>
    </form>
  )
}
