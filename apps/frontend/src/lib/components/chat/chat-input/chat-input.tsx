import React, { type KeyboardEvent, useEffect, useRef } from 'react'
import { Textarea } from '@/lib/components/ui/textarea.tsx'
import { cn } from '@/lib/utils'

interface ChatInputProps {
  message: string
  onMessageChange: (message: string) => void
  onSubmit: () => void
  placeholder?: string
  className?: string
  loading?: boolean
  canSubmit?: boolean
  children?: React.ReactNode
  // File autocomplete support
  onKeyDownCapture?: (e: KeyboardEvent<HTMLTextAreaElement>) => void
  textareaRef?: React.RefObject<HTMLTextAreaElement | null>
  autocompleteSlot?: React.ReactNode
  // Image paste support
  onPaste?: (e: React.ClipboardEvent<HTMLTextAreaElement>) => void
}

export function ChatInput({
  message,
  onMessageChange,
  onSubmit,
  placeholder = 'Type a message...',
  className,
  loading = false,
  canSubmit = true,
  children,
  onKeyDownCapture,
  textareaRef: externalTextareaRef,
  autocompleteSlot,
  onPaste,
}: ChatInputProps) {
  const formRef = useRef<HTMLFormElement>(null)
  const internalTextareaRef = useRef<HTMLTextAreaElement>(null)

  // Use external ref if provided, otherwise use internal ref
  const textareaRef = externalTextareaRef || internalTextareaRef

  // Reset textarea height when message is cleared
  useEffect(() => {
    if (!message && textareaRef.current) {
      textareaRef.current.style.height = 'auto'
    }
  }, [message, textareaRef])

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Call external key handler first (for autocomplete)
    onKeyDownCapture?.(e)

    // If event was handled by autocomplete, don't process further
    if (e.defaultPrevented) {
      return
    }

    if (e.key === 'Enter' && !e.shiftKey && !loading) {
      e.preventDefault()
      if (canSubmit) {
        onSubmit()
      }
    }
  }

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value
    onMessageChange(value)

    const textarea = e.target
    textarea.style.height = 'auto'
    const scrollHeight = textarea.scrollHeight
    textarea.style.height = `${Math.min(scrollHeight, 400)}px`
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!loading && canSubmit) {
      onSubmit()
    }
  }

  const containerStyle =
    'group flex flex-col gap-2 rounded-3xl border border-border bg-card p-2.5 relative'

  return (
    <div className="sticky bottom-0 z-10 flex-shrink-0 bg-background">
      <div data-testid="chat-input" className="p-4">
        <form className={cn(containerStyle, className)} onSubmit={handleSubmit} ref={formRef}>
          <div className="relative flex flex-1 items-center">
            <Textarea
              ref={textareaRef}
              placeholder={placeholder}
              value={message}
              onChange={handleTextareaChange}
              onKeyDown={handleKeyDown}
              onPaste={onPaste}
              className="flex w-full ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none text-[16px] leading-snug placeholder-shown:text-ellipsis placeholder-shown:whitespace-nowrap md:text-base focus-visible:ring-0 focus-visible:ring-offset-0 scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted bg-transparent focus:bg-transparent flex-1 m-1 rounded-md p-0 border-0 shadow-none overflow-y-auto max-h-[400px]"
              disabled={loading}
            />
            {autocompleteSlot}
          </div>

          {children}
        </form>
      </div>
    </div>
  )
}
