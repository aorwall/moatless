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
    // Prevent form submission on Enter unless explicitly allowed
    // Note: For textareas, we typically allow Enter for new lines, but prevent Ctrl+Enter or similar
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !allowFormSubmit) {
      e.preventDefault()
    }

    // Call original onKeyDown if provided
    onKeyDown?.(e)
  }

  return <Textarea className="bg-input min-h-[80px]" onKeyDown={handleKeyDown} {...props} />
}
