import { GitBranch, Plus, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useRepositoriesList } from '@/features/repositories'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'

/** A draft additional repository placement captured during task creation. */
export interface AdditionalRepoDraft {
  repositoryId: string
  branch: string
  mountName: string
}

interface AdditionalReposFieldProps {
  /** The primary repo (the repo selector's value); excluded from the picker. */
  primaryRepositoryId: string
  /** Templates scaffold a single new repo — the affordance is hidden for them. */
  isTemplate: boolean
  /** Matches the composer's centered title layout when set. */
  showTitle?: boolean
  value: AdditionalRepoDraft[]
  onChange: (next: AdditionalRepoDraft[]) => void
  disabled?: boolean
}

/**
 * "Add another repo" affordance for task creation. Empty by default so the
 * single-repo path stays unchanged — only when the user attaches extra repos
 * does the composer build a multi-repo workspace for the task. Renders nothing
 * until a (non-template) primary repository is selected.
 */
export function AdditionalReposField({
  primaryRepositoryId,
  isTemplate,
  showTitle,
  value,
  onChange,
  disabled,
}: AdditionalReposFieldProps) {
  const { data: repositories } = useRepositoriesList()
  const [isAddOpen, setIsAddOpen] = useState(false)

  const repoName = useMemo(() => {
    const byId = new Map(repositories?.map((r) => [r.id, r.name]))
    return (id: string) => byId.get(id) ?? id
  }, [repositories])

  // Repos selectable as additional placements: not templates, not the primary,
  // and not already added.
  const placedIds = useMemo(
    () => new Set([primaryRepositoryId, ...value.map((v) => v.repositoryId)]),
    [primaryRepositoryId, value],
  )
  const available = useMemo(
    () => (repositories ?? []).filter((r) => r.kind !== 'template' && !placedIds.has(r.id)),
    [repositories, placedIds],
  )

  const handleAdd = (draft: AdditionalRepoDraft) => {
    onChange([...value, draft])
    setIsAddOpen(false)
  }

  const handleRemove = (repositoryId: string) => {
    onChange(value.filter((v) => v.repositoryId !== repositoryId))
  }

  // Hidden until a real (non-template) primary repo is chosen, so the
  // single-repo composer looks unchanged until the user opts in.
  if (isTemplate || !primaryRepositoryId) {
    return null
  }

  const containerClass = showTitle ? 'mx-auto mt-2 max-w-2xl' : 'mt-2'

  return (
    <div className={containerClass}>
      <div className="flex flex-wrap items-center gap-2 px-1">
        {value.map((repo) => (
          <Badge key={repo.repositoryId} variant="secondary" className="gap-1 py-1 pr-1 pl-2">
            <span className="max-w-[12rem] truncate">{repoName(repo.repositoryId)}</span>
            {repo.branch && (
              <span className="flex items-center gap-0.5 text-muted-foreground">
                <GitBranch className="h-3 w-3" />
                {repo.branch}
              </span>
            )}
            {repo.mountName && <span className="text-muted-foreground">./{repo.mountName}</span>}
            <button
              type="button"
              aria-label={`Remove ${repoName(repo.repositoryId)}`}
              className="ml-0.5 rounded-sm p-0.5 hover:bg-muted-foreground/20"
              onClick={() => handleRemove(repo.repositoryId)}
              disabled={disabled}
            >
              <X className="h-3 w-3" />
            </button>
          </Badge>
        ))}

        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 gap-1 text-muted-foreground text-xs"
          onClick={() => setIsAddOpen(true)}
          disabled={disabled}
        >
          <Plus className="h-3.5 w-3.5" />
          Add repo
        </Button>

        <AddAdditionalRepoDialog
          open={isAddOpen}
          onOpenChange={setIsAddOpen}
          available={available}
          onAdd={handleAdd}
        />
      </div>
    </div>
  )
}

interface AddAdditionalRepoDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  available: { id: string; name: string }[]
  onAdd: (draft: AdditionalRepoDraft) => void
}

function AddAdditionalRepoDialog({
  open,
  onOpenChange,
  available,
  onAdd,
}: AddAdditionalRepoDialogProps) {
  const [repositoryId, setRepositoryId] = useState('')
  const [branch, setBranch] = useState('')
  const [mountName, setMountName] = useState('')

  const handleOpenChange = (next: boolean) => {
    if (next) {
      setRepositoryId('')
      setBranch('')
      setMountName('')
    }
    onOpenChange(next)
  }

  const handleAdd = () => {
    if (!repositoryId) {
      return
    }
    onAdd({ repositoryId, branch: branch.trim(), mountName: mountName.trim() })
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add another repository</DialogTitle>
          <DialogDescription>
            Check an extra repository out alongside the primary one. Branch and mount name are
            optional and default to the repository's default branch and name.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label>Repository</Label>
            <Select value={repositoryId} onValueChange={setRepositoryId}>
              <SelectTrigger>
                <SelectValue placeholder="Select a repository" />
              </SelectTrigger>
              <SelectContent>
                {available.length === 0 ? (
                  <div className="px-2 py-1.5 text-muted-foreground text-sm">
                    No repositories available
                  </div>
                ) : (
                  available.map((repo) => (
                    <SelectItem key={repo.id} value={repo.id}>
                      {repo.name}
                    </SelectItem>
                  ))
                )}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label>Branch</Label>
            <Input
              value={branch}
              onChange={(e) => setBranch(e.target.value)}
              placeholder="(repository default)"
            />
          </div>

          <div className="space-y-2">
            <Label>Mount name</Label>
            <Input
              value={mountName}
              onChange={(e) => setMountName(e.target.value)}
              placeholder="(repository name)"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleAdd} disabled={!repositoryId}>
            Add repository
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
