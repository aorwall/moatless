import type { RepositoryResponse } from '@moatless/types'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select.tsx'

interface RepositorySelectorProps {
  repositories?: Array<RepositoryResponse>
  selectedRepositoryId?: string
  onRepositoryChange: (repositoryId: string) => void
  disabled?: boolean
  loading?: boolean
}

export function RepositorySelector({
  repositories,
  selectedRepositoryId,
  onRepositoryChange,
  disabled = false,
  loading = false,
}: RepositorySelectorProps) {
  return (
    <Select
      value={selectedRepositoryId}
      onValueChange={onRepositoryChange}
      disabled={disabled || loading}
    >
      <SelectTrigger className="w-full sm:w-64">
        <SelectValue placeholder="Select repository (required)" />
      </SelectTrigger>
      <SelectContent>
        {repositories?.map((repo) => (
          <SelectItem key={repo.id} value={repo.id}>
            {repo.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
