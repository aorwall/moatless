import { useId } from 'react'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Switch } from '@/lib/components/ui/switch'

interface LocalRepositoryConfigProps {
  path: string
  useGitWorktrees: boolean
  onPathChange: (path: string) => void
  onUseGitWorktreesChange: (useGitWorktrees: boolean) => void
  pathError?: string
}

export function LocalRepositoryConfig({
  path,
  useGitWorktrees,
  onPathChange,
  onUseGitWorktreesChange,
  pathError,
}: LocalRepositoryConfigProps) {
  const pathId = useId()
  const worktreesId = useId()

  return (
    <div className="space-y-4">
      <div>
        <Label htmlFor={pathId}>Repository Path</Label>
        <Input
          id={pathId}
          value={path}
          onChange={(e) => onPathChange(e.target.value)}
          placeholder="/path/to/local/repository"
          required
          className={pathError ? 'border-destructive' : ''}
        />
        {pathError && <p className="text-sm text-destructive mt-1">{pathError}</p>}
        <p className="text-sm text-muted-foreground mt-1">
          Absolute path to the local repository directory
        </p>
      </div>

      <div className="flex items-center space-x-2">
        <Switch
          id={worktreesId}
          checked={useGitWorktrees}
          onCheckedChange={onUseGitWorktreesChange}
        />
        <Label htmlFor={worktreesId}>Use Git Worktrees</Label>
      </div>
      <p className="text-sm text-muted-foreground">
        Create Git worktrees for each session to isolate changes
      </p>
    </div>
  )
}
