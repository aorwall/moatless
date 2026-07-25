import { Scope, type SecretMetadataResponse } from '@moatless/api-client'
import { Plus } from 'lucide-react'
import { useState } from 'react'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { SecretEditorDialog } from '@/features/secrets/components/secret-editor-dialog'
import { SecretsList } from '@/features/secrets/components/secrets-list'
import { useSecretsList } from '@/features/secrets/hooks/use-secrets'
import { Button } from '@/lib/components/ui/button'
import { Tabs, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'

type EditorScope = 'user' | 'global'

interface SecretsPanelProps {
  /** Scope tab to select on mount. Defaults to the current user's own secrets. */
  defaultScope?: EditorScope
}

export function SecretsPanel({ defaultScope = 'user' }: SecretsPanelProps = {}) {
  const { isAdmin } = useAuth()
  const [scope, setScope] = useState<EditorScope>(defaultScope)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editSecret, setEditSecret] = useState<SecretMetadataResponse | null>(null)

  const { data: secrets, isLoading, error } = useSecretsList({ scope })

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
          <TabsTrigger value={Scope.user}>User</TabsTrigger>
          <TabsTrigger value={Scope.global} disabled={!isAdmin}>
            Global
          </TabsTrigger>
        </TabsList>
      </Tabs>

      <SecretsList
        secrets={secrets ?? []}
        isLoading={isLoading}
        error={error as Error | null}
        onEdit={openEdit}
      />

      <SecretEditorDialog
        key={`${dialogOpen ? 'open' : 'closed'}:${editSecret?.id ?? 'new'}:${scope}`}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        editSecret={editSecret}
        defaultScope={scope}
      />
    </div>
  )
}
