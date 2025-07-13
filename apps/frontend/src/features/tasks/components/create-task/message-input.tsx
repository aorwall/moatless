import { ArrowUp, Loader2 } from 'lucide-react'
import type { KeyboardEvent, ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { FileAutocomplete } from '@/features/tasks/components/file-autocomplete'
import { Button } from '@/lib/components/ui/button'
import { Textarea } from '@/lib/components/ui/textarea'
import { useUniversalFileAutocomplete } from '@/lib/hooks/use-universal-file-autocomplete'

interface MessageInputProps {
  value: string
  onChange: (value: string) => void
  onSubmit: () => void
  placeholder?: string
  disabled?: boolean
  loading?: boolean
  buttonText?: string
  maxHeight?: string
  selectors?: ReactNode
  repositoryId?: string // For file autocomplete
  taskId?: string // For file autocomplete in task context
  isTaskContext?: boolean // For dropdown positioning
}

export function MessageInput({
  value,
  onChange,
  onSubmit,
  placeholder = 'Type your message...',
  disabled = false,
  loading = false,
  buttonText = 'Send',
  maxHeight = '200px',
  selectors,
  repositoryId,
  taskId,
  isTaskContext = false,
}: MessageInputProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const [dropdownPosition, setDropdownPosition] = useState<'above' | 'below'>('below')

  // File autocomplete functionality - always call hook but disable when no repositoryId
  const {
    inputRef: autocompleteInputRef,
    isOpen: isAutocompleteOpen,
    results: autocompleteResults,
    selectedIndex,
    isLoading: isAutocompleteLoading,
    error: autocompleteError,
    handleInputChange: handleAutocompleteInputChange,
    handleKeyDown: handleAutocompleteKeyDown,
    selectFile,
  } = useUniversalFileAutocomplete({
    repositoryId: repositoryId || '', // Provide empty string as fallback
    taskId,
    value,
    onValueChange: onChange,
  })

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Let autocomplete handle key events first if it's open
    if (isAutocompleteOpen && repositoryId) {
      handleAutocompleteKeyDown(e)
      if (['ArrowDown', 'ArrowUp', 'Enter', 'Escape', 'Tab'].includes(e.key)) {
        return
      }
    }

    if (e.key === 'Enter' && !e.shiftKey && !isAutocompleteOpen) {
      e.preventDefault()
      if (!disabled && value.trim()) {
        onSubmit()
      }
    }
  }

  // Callback ref to handle initial resize and ref assignment
  const textareaCallbackRef = (node: HTMLTextAreaElement | null) => {
    // Assign to both refs
    textareaRef.current = node
    if (autocompleteInputRef) {
      autocompleteInputRef.current = node
    }

    if (node) {
      // Set initial height using callback ref pattern
      node.style.height = 'auto'
      const scrollHeight = node.scrollHeight
      const maxHeightPx = parseInt(maxHeight)
      const newHeight = Math.min(scrollHeight, maxHeightPx)
      node.style.height = `${newHeight}px`
    }
  }

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const newValue = e.target.value
    const cursorPosition = e.target.selectionStart || 0
    onChange(newValue)

    // Handle file autocomplete if repositoryId is provided
    if (repositoryId) {
      handleAutocompleteInputChange(newValue, cursorPosition)
    }

    // Auto-resize textarea on input
    const textarea = e.target
    textarea.style.height = 'auto'
    const scrollHeight = textarea.scrollHeight
    const maxHeightPx = parseInt(maxHeight)
    const newHeight = Math.min(scrollHeight, maxHeightPx)
    textarea.style.height = `${newHeight}px`
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

  return (
    <div className="group flex flex-col gap-2 rounded-3xl border border-border bg-muted p-3">
      <div ref={containerRef}>
        <div className="relative flex flex-1 items-center">
          <Textarea
            ref={textareaCallbackRef}
            value={value}
            onChange={handleTextareaChange}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            disabled={loading}
            className="flex w-full ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none text-[16px] leading-snug placeholder-shown:text-ellipsis placeholder-shown:whitespace-nowrap md:text-base focus-visible:ring-0 focus-visible:ring-offset-0 scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted-foreground bg-transparent focus:bg-transparent flex-1 m-1 rounded-md p-0 border-0 shadow-none overflow-y-auto"
            style={{ minHeight: '40px', maxHeight }}
          />
          {isAutocompleteOpen && repositoryId && (
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
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">{selectors}</div>
        <Button
          onClick={onSubmit}
          disabled={disabled || !value.trim()}
          size="icon"
          className="flex size-6 items-center justify-center rounded-full bg-foreground text-background transition-opacity duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 w-6"
        >
          {loading ? <Loader2 className="h-5 w-5 animate-spin" /> : <ArrowUp className="h-5 w-5" />}
          <span className="sr-only">{buttonText}</span>
        </Button>
      </div>
    </div>
  )
}
