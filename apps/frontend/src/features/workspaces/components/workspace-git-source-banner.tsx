import type { WorkspaceResponse } from '@moatless/api-client'
import { GitBranch, Loader2, PencilLine, RotateCcw } from 'lucide-react'
import { useRepository } from '@/features/repositories/hooks/use-repositories'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'

interface WorkspaceGitSourceBannerProps {
  workspace: WorkspaceResponse
  onOverride: () => void
  onReset: () => void
  isOverriding: boolean
  isResetting: boolean
}

/**
 * Provenance banner for a workspace declared in a repository's git tree.
 *
 * Renders nothing for a plain manual workspace. A git-synced one offers
 * "Override to edit"; an overridden one offers "Reset to Git", warning that the
 * local changes are replaced rather than merged.
 */
export function WorkspaceGitSourceBanner({
  workspace,
  onOverride,
  onReset,
  isOverriding,
  isResetting,
}: WorkspaceGitSourceBannerProps) {
  const sourceRepositoryId = workspace.sourceRepositoryId ?? ''
  const { data: sourceRepo } = useRepository(sourceRepositoryId)

  // Provenance survives an override, so it is what distinguishes a workspace
  // taken out of git's hands from one that was never in it.
  if (!sourceRepositoryId) {
    return null
  }

  const origin = `${sourceRepo?.name ?? sourceRepositoryId}/${
    workspace.sourceConfigPath ?? '.moatless/workspaces.json'
  }`

  if (workspace.syncedFromGit) {
    return (
      <Alert>
        <GitBranch className="h-4 w-4" />
        <AlertTitle>Synced from Git</AlertTitle>
        <AlertDescription className="flex flex-col items-start gap-3">
          <span>
            This workspace is defined in {origin} and is read-only. Edit the file in Git, or
            override it here to make this copy editable — it will stop syncing until you reset it.
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onOverride}
            disabled={isOverriding}
          >
            {isOverriding ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Overriding...
              </>
            ) : (
              <>
                <PencilLine className="mr-2 h-4 w-4" />
                Override to edit
              </>
            )}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <Alert>
      <PencilLine className="h-4 w-4" />
      <AlertTitle>Overridden — no longer synced from Git</AlertTitle>
      <AlertDescription className="flex flex-col items-start gap-3">
        <span>
          This workspace came from {origin} and is now edited here. Warning: resetting replaces
          these local changes — including repository placements — with the committed config.
        </span>
        <Button type="button" variant="outline" size="sm" onClick={onReset} disabled={isResetting}>
          {isResetting ? (
            <>
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              Resetting...
            </>
          ) : (
            <>
              <RotateCcw className="mr-2 h-4 w-4" />
              Reset to Git
            </>
          )}
        </Button>
      </AlertDescription>
    </Alert>
  )
}
