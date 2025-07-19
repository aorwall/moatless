import type { InputHTMLAttributes } from 'react'
import { Input } from '@/lib/components/ui/input'

export interface ConfigurationInputProps extends InputHTMLAttributes<HTMLInputElement> {}

export function ConfigurationInput(props: ConfigurationInputProps) {
  return <Input className="bg-input" {...props} />
}
