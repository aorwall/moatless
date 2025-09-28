import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select.tsx'
import { cn } from '@/lib/utils.ts'

// Common props all selectors share
export interface BaseSelectorProps {
  value?: string
  onChange: (value: string) => void
  loading?: boolean
  disabled?: boolean
  className?: string
  error?: Error | null
  isError?: boolean
}

export interface GenericSelectorProps extends BaseSelectorProps {
  placeholder: string
  items: Array<{ id: string; label: string }>
  width?: string
}

export function GenericSelector({
  value,
  onChange,
  loading,
  disabled,
  placeholder,
  items,
  width = 'w-[180px]',
  className,
  error,
  isError,
}: GenericSelectorProps) {
  const getPlaceholder = () => {
    if (loading) return 'Loading...'
    if (isError) return 'Failed to load'
    return placeholder
  }

  const getTriggerClassName = () => {
    const base = 'h-6 px-2 text-xs'
    const errorClass = isError ? 'border-destructive text-destructive' : ''
    return cn(base, width, errorClass, className)
  }

  return (
    <Select value={value} onValueChange={onChange} disabled={disabled || loading || isError}>
      <SelectTrigger size="sm" className={getTriggerClassName()}>
        <SelectValue placeholder={getPlaceholder()} />
      </SelectTrigger>
      <SelectContent>
        {isError ? (
          <SelectItem value="__error__" disabled className="text-xs text-destructive">
            {error?.message || 'Failed to load options'}
          </SelectItem>
        ) : (
          items.map((item) => (
            <SelectItem key={item.id} value={item.id} className="text-xs">
              {item.label}
            </SelectItem>
          ))
        )}
      </SelectContent>
    </Select>
  )
}
