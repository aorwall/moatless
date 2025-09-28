import type { KeyboardEvent, TextareaHTMLAttributes } from 'react'
import { Textarea } from '@/lib/components/ui/textarea'

export interface ConfigurationTextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  allowFormSubmit?: boolean
}

export function ConfigurationTextarea({
  allowFormSubmit = false,
  onKeyDown,
  ...props
}: ConfigurationTextareaProps) {
  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // For textareas, we need to handle different Enter key scenarios:
    // 1. Regular Enter should always create a new line (don't prevent)
    // 2. Ctrl+Enter should be prevented from submitting forms unless explicitly allowed
    // 3. The key insight is to NOT prevent regular Enter, but stop propagation to prevent form submission

    if (e.key === 'Enter') {
      if (e.ctrlKey || e.metaKey) {
        // Ctrl+Enter or Cmd+Enter: prevent form submission unless allowed
        if (!allowFormSubmit) {
          e.preventDefault()
        }
      } else {
        // Regular Enter: stop propagation to prevent form submission but allow textarea to handle it
        e.stopPropagation()
      }
    }

    // Call original onKeyDown if provided
    onKeyDown?.(e)
  }

  return <Textarea className="min-h-[80px]" onKeyDown={handleKeyDown} {...props} />
}
