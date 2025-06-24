import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import type { Environment } from '@/types/environment'

interface EnvironmentSelectorProps {
  environments?: Array<Environment>
  selectedEnvironmentId?: string
  onEnvironmentChange: (environmentId: string) => void
  disabled?: boolean
  loading?: boolean
}

export function EnvironmentSelector({
  environments,
  selectedEnvironmentId,
  onEnvironmentChange,
  disabled = false,
  loading = false,
}: EnvironmentSelectorProps) {
  return (
    <Select
      value={selectedEnvironmentId}
      onValueChange={onEnvironmentChange}
      disabled={disabled || loading}
    >
      <SelectTrigger className="w-full sm:w-64">
        <SelectValue placeholder="Select environment (required)" />
      </SelectTrigger>
      <SelectContent>
        {environments?.map((env) => (
          <SelectItem key={env.id} value={env.id}>
            {env.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
