import type { TextareaHTMLAttributes } from 'react'
import { Textarea } from '@/lib/components/ui/textarea'

export interface ConfigurationTextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {}

export function ConfigurationTextarea(props: ConfigurationTextareaProps) {
  return <Textarea className="bg-input min-h-[80px]" {...props} />
}
