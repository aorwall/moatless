import { CLAUDE_MODEL_DISPLAY_NAMES, CLAUDE_MODELS, type ClaudeModel } from '@shared/constants'
import type { UploadFileResponse } from '@shared/types/api'
import type { Environment } from '@shared/types/environment'
import { ArrowUp, Circle, Paperclip, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { FileAutocomplete } from '@/features/tasks/components/file-autocomplete'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import { Button } from '@/lib/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import { Textarea } from '@/lib/components/ui/textarea'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { useUniversalFileAutocomplete } from '@/lib/hooks/use-universal-file-autocomplete'

interface ChatFormProps {
  message: string
  onMessageChange: (message: string) => void
  sendMessage: () => void
  textareaDisabled?: boolean
  sendDisabled?: boolean
  placeholder?: string
  environmentId?: string
  taskId?: string
  isTaskContext: boolean
  buttonText: string
  selectedModel: ClaudeModel
  onModelChange: (model: ClaudeModel) => void
  isRunning?: boolean
  // Environment selector props (only shown when creating new task)
  showEnvironmentSelector?: boolean
  environments?: Environment[]
  selectedEnvironmentId?: string
  onEnvironmentChange?: (environmentId: string) => void
  environmentsLoading?: boolean
  maxHeight?: string
  uploadedFiles?: UploadFileResponse[]
  onFilesChange?: (files: UploadFileResponse[]) => void
}

export function ChatForm({
  message,
  onMessageChange,
  sendMessage,
  textareaDisabled = false,
  sendDisabled = false,
  placeholder,
  environmentId,
  taskId,
  isTaskContext,
  buttonText,
  selectedModel,
  onModelChange,
  isRunning = false,
  showEnvironmentSelector = false,
  environments,
  selectedEnvironmentId,
  onEnvironmentChange,
  environmentsLoading = false,
  maxHeight = '200px',
  uploadedFiles = [],
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
    environmentId,
    taskId,
    value: message,
    onValueChange: onMessageChange,
  })

  // File upload functionality
  const uploadFile = useUploadFile({
    onSuccess: (data) => {
      if (onFilesChange) {
        onFilesChange([...uploadedFiles, data])
      }
    },
    onError: (error) => {
      console.error('File upload failed:', error)
      // TODO: Show error toast/notification
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
            onFilesChange([...uploadedFiles, data])
          }
        },
      })
    }
    // Reset input so same file can be selected again
    e.target.value = ''
  }

  const removeFile = (id: string) => {
    if (onFilesChange) {
      onFilesChange(uploadedFiles.filter((f) => f.id !== id))
    }
  }

  // Initial resize on mount
  useEffect(() => {
    const textarea = inputRef.current
    if (!textarea) return

    // Set initial height
    textarea.style.height = 'auto'
    const scrollHeight = textarea.scrollHeight
    const maxHeightPx = parseInt(maxHeight)
    const newHeight = Math.min(scrollHeight, maxHeightPx)
    textarea.style.height = `${newHeight}px`
  }, [maxHeight, inputRef.current])

  // Detect if dropdown should appear above or below
  useEffect(() => {
    if (isAutocompleteOpen && containerRef.current) {
      const rect = containerRef.current.getBoundingClientRect()
      const spaceBelow = window.innerHeight - rect.bottom
      const dropdownHeight = 240 // max-h-60 = 15rem = 240px

      // If not enough space below and we're in task context (likely at bottom), show above
      if (spaceBelow < dropdownHeight && isTaskContext) {
        setDropdownPosition('above')
      } else {
        setDropdownPosition('below')
      }
    }
  }, [isAutocompleteOpen, isTaskContext])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Let autocomplete handle key events first
    if (isAutocompleteOpen) {
      handleAutocompleteKeyDown(e)
      if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'].includes(e.key)) {
        return
      }
    }

    if (e.key === 'Enter' && !e.shiftKey && !isAutocompleteOpen && !sendDisabled) {
      e.preventDefault()
      sendMessage()
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
    const maxHeightPx = parseInt(maxHeight)
    const newHeight = Math.min(scrollHeight, maxHeightPx)
    textarea.style.height = `${newHeight}px`
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!sendDisabled) {
      sendMessage()
    }
  }

  return (
    <form
      className="duration-800 group flex flex-col gap-2 rounded-3xl border border-border bg-muted p-3 transition-colors ease-in-out relative"
      onSubmit={handleSubmit}
      ref={formRef}
    >
      <div ref={containerRef}>
        <div className="relative flex flex-1 items-center">
          <Textarea
            ref={inputRef}
            placeholder={placeholder}
            value={message}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            className="flex w-full ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none text-[16px] leading-snug placeholder-shown:text-ellipsis placeholder-shown:whitespace-nowrap md:text-base focus-visible:ring-0 focus-visible:ring-offset-0 scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted-foreground bg-transparent focus:bg-transparent flex-1 m-1 rounded-md p-0 border-0 shadow-none overflow-y-auto"
            style={{ minHeight: '40px', maxHeight }}
            disabled={textareaDisabled}
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
      {uploadedFiles.length > 0 && (
        <div className="flex flex-wrap gap-2 mb-2">
          {uploadedFiles.map((file) => {
            const isImage = file.mimeType.startsWith('image/')
            // Use the upload endpoint to get the image
            const imageUrl = isImage ? `/api/v1/uploads/${file.id}/${file.filename}` : null

            return (
              <div
                key={file.id}
                className="flex items-center gap-1 px-2 py-1 bg-muted-foreground/10 rounded text-xs"
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
                  className="text-muted-foreground hover:text-foreground"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            )
          })}
        </div>
      )}
      <div className="flex items-center justify-between gap-2">
        {/* Left side - Environment and Model selectors */}
        <div className="flex items-center gap-2">
          {showEnvironmentSelector && onEnvironmentChange && (
            <Select
              value={selectedEnvironmentId}
              onValueChange={onEnvironmentChange}
              disabled={environmentsLoading}
            >
              <SelectTrigger size="sm" className="h-6 px-2 text-xs w-[180px]">
                <SelectValue placeholder="Select environment" />
              </SelectTrigger>
              <SelectContent>
                {environments?.map((env) => (
                  <SelectItem key={env.id} value={env.id} className="text-xs">
                    {env.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          <Select value={selectedModel} onValueChange={onModelChange}>
            <SelectTrigger size="sm" className="h-6 px-2 text-xs w-[130px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={CLAUDE_MODELS.OPUS_4} className="text-xs">
                {CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS_4]}
              </SelectItem>
              <SelectItem value={CLAUDE_MODELS.SONNET_4} className="text-xs">
                {CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET_4]}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>

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
          {isRunning ? (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    type="submit"
                    disabled={sendDisabled}
                    size="sm"
                    className="flex items-center justify-center gap-1.5 rounded-full bg-transparent border border-gray-400/30 text-gray-500 transition-all duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 px-3 text-xs font-medium hover:bg-gray-500/10 hover:border-gray-400/40"
                  >
                    <span>{buttonText}</span>
                    <Circle className="h-3 w-3 text-gray-500" />
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
              disabled={sendDisabled}
              size="icon"
              className="flex size-6 items-center justify-center rounded-full bg-foreground text-background transition-opacity duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 w-6"
            >
              <ArrowUp className="h-5 w-5" />
              <span className="sr-only">{buttonText}</span>
            </Button>
          )}
        </div>
      </div>
    </form>
  )
}
