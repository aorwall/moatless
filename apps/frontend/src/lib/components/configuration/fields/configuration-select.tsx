import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

interface ConfigurationSelectProps {
  value?: string
  onValueChange?: (value: string) => void
  options: { value: string; label: string }[]
  placeholder?: string
  disabled?: boolean
}

export function ConfigurationSelect({
  value,
  onValueChange,
  options,
  placeholder,
  disabled,
}: ConfigurationSelectProps) {
  return (
    <Select
      {...(value !== undefined && { value })}
      {...(onValueChange !== undefined && { onValueChange })}
      {...(disabled !== undefined && { disabled })}
    >
      <SelectTrigger>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
