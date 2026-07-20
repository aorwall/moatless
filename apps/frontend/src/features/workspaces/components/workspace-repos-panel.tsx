import type { WorkspaceRepoResponse } from '@moatless/api-client'
import { GitBranch, Loader2, Plus, Star, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import {
  useAddWorkspaceRepo,
  useRemoveWorkspaceRepo,
  useSetWorkspacePrimaryRepo,
} from '@/features/workspaces/hooks/use-workspaces'
import { ConfigurationGroup } from '@/lib/components/configuration'
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
import { getErrorMessage } from '@/lib/utils/error'

interface WorkspaceReposPanelProps {
  workspaceId: string
  placements: WorkspaceRepoResponse[]
}

/**
 * Repository placements for a workspace: add / remove / set-primary, with
 * per-placement branch + mount name captured at add time.
 *
 * Branch and mount name are set when a repo is placed and shown read-only
 * afterwards — the composition API has no placement-update endpoint, so
 * changing them means removing and re-adding the placement. (Same rationale as
 * dropping reorder: we don't fake operations the backend can't do.)
 */
export function WorkspaceReposPanel({ workspaceId, placements }: WorkspaceReposPanelProps) {
  const { data: repositories } = useRepositoriesList()
  const addRepo = useAddWorkspaceRepo()
  const removeRepo = useRemoveWorkspaceRepo()
  const setPrimary = useSetWorkspacePrimaryRepo()
  const [isAddOpen, setIsAddOpen] = useState(false)

  const repoName = useMemo(() => {
    const byId = new Map(repositories?.map((r) => [r.id, r.name]))
    return (id: string) => byId.get(id) ?? id
  }, [repositories])

  const handleRemove = (placement: WorkspaceRepoResponse) => {
    removeRepo.mutate(
      { workspaceId, placementId: placement.id },
      {
        onError: (error) =>
          toast.error('Failed to remove repository', { description: getErrorMessage(error) }),
      },
    )
  }

  const handleSetPrimary = (placement: WorkspaceRepoResponse) => {
    setPrimary.mutate(
      { workspaceId, placementId: placement.id },
      {
        onError: (error) =>
          toast.error('Failed to set primary repository', {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  return (
    <ConfigurationGroup title="Repositories" showSeparator>
      <p className="text-muted-foreground text-sm">
        Repositories checked out into this workspace. The primary repo owns the diff, PR, and
        CLAUDE.md.
      </p>
      <div className="space-y-2">
        {placements.length === 0 ? (
          <div className="flex flex-col items-center justify-center rounded-md border border-dashed py-8 text-muted-foreground">
            <p className="text-sm">No repositories</p>
            <p className="mt-1 text-xs">
              This is a blank workspace. Add a repository to check out.
            </p>
          </div>
        ) : (
          placements.map((placement) => (
            <div
              key={placement.id}
              className="flex items-center gap-3 rounded-md border bg-card p-3"
            >
              <div className="flex min-w-0 flex-1 items-center gap-2">
                {placement.isPrimary && (
                  <Star className="h-4 w-4 shrink-0 fill-amber-400 text-amber-400" />
                )}
                <span className="truncate font-medium text-sm">
                  {repoName(placement.repositoryId)}
                </span>
                {placement.isPrimary && (
                  <Badge variant="secondary" className="shrink-0 text-xs">
                    Primary
                  </Badge>
                )}
                {placement.branch && (
                  <span className="flex shrink-0 items-center gap-1 text-muted-foreground text-xs">
                    <GitBranch className="h-3 w-3" />
                    {placement.branch}
                  </span>
                )}
                {placement.mountName && (
                  <span className="truncate text-muted-foreground text-xs">
                    ./{placement.mountName}
                  </span>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {!placement.isPrimary && (
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    title="Set as primary"
                    disabled={setPrimary.isPending}
                    onClick={() => handleSetPrimary(placement)}
                  >
                    <Star className="h-3.5 w-3.5 text-muted-foreground hover:text-amber-400" />
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  aria-label={`Remove ${repoName(placement.repositoryId)}`}
                  disabled={removeRepo.isPending}
                  onClick={() => handleRemove(placement)}
                >
                  <Trash2 className="h-3.5 w-3.5 text-muted-foreground hover:text-destructive" />
                </Button>
              </div>
            </div>
          ))
        )}

        <Button variant="outline" size="sm" onClick={() => setIsAddOpen(true)}>
          <Plus className="h-4 w-4" />
          Add Repository
        </Button>
      </div>

      <AddRepoDialog
        open={isAddOpen}
        onOpenChange={setIsAddOpen}
        repositories={(repositories ?? []).filter((r) => r.kind !== 'template')}
        placedRepositoryIds={placements.map((p) => p.repositoryId)}
        hasPrimary={placements.some((p) => p.isPrimary)}
        isSaving={addRepo.isPending}
        onAdd={(request) =>
          addRepo.mutate(
            { workspaceId, request },
            {
              onSuccess: () => setIsAddOpen(false),
              onError: (error) =>
                toast.error('Failed to add repository', { description: getErrorMessage(error) }),
            },
          )
        }
      />
    </ConfigurationGroup>
  )
}

interface AddRepoDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  repositories: { id: string; name: string }[]
  placedRepositoryIds: string[]
  hasPrimary: boolean
  isSaving: boolean
  onAdd: (request: {
    repositoryId: string
    branch?: string | null
    mountName?: string | null
    isPrimary?: boolean | null
  }) => void
}

function AddRepoDialog({
  open,
  onOpenChange,
  repositories,
  placedRepositoryIds,
  hasPrimary,
  isSaving,
  onAdd,
}: AddRepoDialogProps) {
  const [repositoryId, setRepositoryId] = useState('')
  const [branch, setBranch] = useState('')
  const [mountName, setMountName] = useState('')

  const available = useMemo(
    () => repositories.filter((r) => !placedRepositoryIds.includes(r.id)),
    [repositories, placedRepositoryIds],
  )

  // Reset the form each time the dialog opens.
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
    onAdd({
      repositoryId,
      branch: branch.trim() || null,
      mountName: mountName.trim() || null,
      // First repo in a workspace with no primary yet becomes primary.
      isPrimary: hasPrimary ? null : true,
    })
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Repository</DialogTitle>
          <DialogDescription>
            Place a repository into this workspace. Branch and mount name are optional and default
            to the repository's default branch and name.
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
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isSaving}>
            Cancel
          </Button>
          <Button onClick={handleAdd} disabled={!repositoryId || isSaving}>
            {isSaving ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Adding...
              </>
            ) : (
              'Add Repository'
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
