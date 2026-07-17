import { type SecretMetadataResponse, SecretScopeType } from '@moatless/api-client'
import { Plus } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { SecretEditorDialog } from '@/features/secrets/components/secret-editor-dialog'
import { SecretsList } from '@/features/secrets/components/secrets-list'
import { TeamSelect } from '@/features/secrets/components/team-select'
import { useSecretsList, useTeamsList } from '@/features/secrets/hooks/use-secrets'
import { Button } from '@/lib/components/ui/button'
import { Label } from '@/lib/components/ui/label'
import { Tabs, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'

type EditorScope = 'user' | 'team' | 'global'

interface SecretsPanelProps {
  /** Scope tab to select on mount. Defaults to the current user's own secrets. */
  defaultScope?: EditorScope
}

export function SecretsPanel({ defaultScope = 'user' }: SecretsPanelProps = {}) {
  const { user, isAdmin } = useAuth()
  const [scope, setScope] = useState<EditorScope>(defaultScope)
  const [teamId, setTeamId] = useState('')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editSecret, setEditSecret] = useState<SecretMetadataResponse | null>(null)
  const {
    data: teams = [],
    isLoading: teamsLoading,
    error: teamsError,
  } = useTeamsList({
    enabled: scope === SecretScopeType.team || dialogOpen,
  })

  const scopeId = useMemo(() => {
    if (scope === SecretScopeType.user) {
      return user?.id ?? undefined
    }
    if (scope === SecretScopeType.team) {
      return teamId.trim() || undefined
    }
    return
  }, [scope, teamId, user?.id])

  const {
    data: secrets,
    isLoading,
    error,
  } = useSecretsList(
    { scope, ...(scopeId !== undefined && { scopeId }) },
    {
      enabled: scope !== SecretScopeType.team || !!scopeId,
    },
  )

  const openCreate = () => {
    setEditSecret(null)
    setDialogOpen(true)
  }

  const openEdit = (secret: SecretMetadataResponse) => {
    setEditSecret(secret)
    setDialogOpen(true)
  }

  return (
    <div className="mx-auto max-w-5xl space-y-6 p-6">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold">Secrets</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Encrypted environment variables and provider tokens. Delivered to sandboxes at task
            start; never returned by the API after save.
          </p>
        </div>
        <Button onClick={openCreate}>
          <Plus className="mr-2 h-4 w-4" />
          New Secret
        </Button>
      </header>

      <Tabs value={scope} onValueChange={(v) => setScope(v as EditorScope)}>
        <TabsList>
          <TabsTrigger value={SecretScopeType.user}>User</TabsTrigger>
          <TabsTrigger value={SecretScopeType.team}>Team</TabsTrigger>
          <TabsTrigger value={SecretScopeType.global} disabled={!isAdmin}>
            Global
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {scope === SecretScopeType.team && (
        <div className="space-y-2">
          <Label htmlFor="secrets-team">Team</Label>
          <TeamSelect
            id="secrets-team"
            value={teamId}
            onValueChange={setTeamId}
            teams={teams}
            isLoading={teamsLoading}
            error={teamsError as Error | null}
          />
        </div>
      )}

      {scope === SecretScopeType.team && !scopeId ? (
        <div className="rounded-md border border-dashed py-12 text-center text-sm text-muted-foreground">
          Select a team to list secrets.
        </div>
      ) : (
        <SecretsList
          secrets={secrets ?? []}
          isLoading={isLoading}
          error={error as Error | null}
          onEdit={openEdit}
        />
      )}

      <SecretEditorDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        editSecret={editSecret}
        defaultScope={scope}
        teams={teams}
        teamsError={teamsError as Error | null}
        teamsLoading={teamsLoading}
      />
    </div>
  )
}
