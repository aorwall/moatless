import { cn } from '@/lib/utils'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

// Common props all selectors share
export interface BaseSelectorProps {
  value?: string
  onChange: (value: string) => void
  loading?: boolean
  disabled?: boolean
  className?: string
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
  width = "w-[180px]",
  className
}: GenericSelectorProps) {
  return (
    <Select
      value={value}
      onValueChange={onChange}
      disabled={disabled || loading}
    >
      <SelectTrigger size="sm" className={cn("h-6 px-2 text-xs", width, className)}>
        <SelectValue placeholder={loading ? "Loading..." : placeholder} />
      </SelectTrigger>
      <SelectContent>
        {items.map((item) => (
          <SelectItem key={item.id} value={item.id} className="text-xs">
            {item.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}