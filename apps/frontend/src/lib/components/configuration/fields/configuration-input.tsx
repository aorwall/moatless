import type { InputHTMLAttributes, KeyboardEvent } from 'react'
import { Input } from '@/lib/components/ui/input'

export interface ConfigurationInputProps extends InputHTMLAttributes<HTMLInputElement> {
  allowFormSubmit?: boolean
}

export function ConfigurationInput({
  allowFormSubmit = false,
  onKeyDown,
  ...props
}: ConfigurationInputProps) {
  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    // Prevent form submission on Enter unless explicitly allowed
    if (e.key === 'Enter' && !allowFormSubmit) {
      e.preventDefault()
    }

    // Call original onKeyDown if provided
    onKeyDown?.(e)
  }

  return <Input onKeyDown={handleKeyDown} {...props} />
}
