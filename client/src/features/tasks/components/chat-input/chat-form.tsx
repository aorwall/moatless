import { ArrowUp } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { FileAutocomplete } from '@/features/tasks/components/file-autocomplete'
import { Button } from '@/lib/components/ui/button'
import { Textarea } from '@/lib/components/ui/textarea'
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
}: ChatFormProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const formRef = useRef<HTMLFormElement>(null)
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
            className="flex w-full ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none text-[16px] leading-snug placeholder-shown:text-ellipsis placeholder-shown:whitespace-nowrap md:text-base focus-visible:ring-0 focus-visible:ring-offset-0 max-h-[200px] scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted-foreground bg-transparent focus:bg-transparent flex-1 m-1 rounded-md p-0 border-0 shadow-none"
            style={{ minHeight: '40px' }}
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
      <div className="flex items-center gap-1">
        <div className="ml-auto flex items-center gap-2">
          <Button
            type="submit"
            disabled={sendDisabled}
            size="icon"
            className="flex size-6 items-center justify-center rounded-full bg-foreground text-background transition-opacity duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 w-6"
          >
            <ArrowUp className="h-5 w-5" />
            <span className="sr-only">{buttonText}</span>
          </Button>
        </div>
      </div>
    </form>
  )
}
