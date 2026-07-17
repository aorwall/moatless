import { SecretKind, type SecretMetadataResponse, SecretScopeType } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { TeamSelect } from '@/features/secrets/components/team-select'
import {
  type PatchSecretRequest,
  type PutSecretRequest,
  type TeamResponse,
  usePatchSecret,
  usePutSecret,
} from '@/features/secrets/hooks/use-secrets'
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
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

type EditorScope = 'user' | 'team' | 'global'

interface SecretEditorDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  editSecret?: SecretMetadataResponse | null
  defaultScope?: EditorScope
  teams?: TeamResponse[]
  teamsError?: Error | null
  teamsLoading?: boolean
}

interface FormState {
  scope: EditorScope
  scopeId: string
  key: string
  kind: SecretKind
  value: string
}

function initialForm(defaultScope: EditorScope): FormState {
  return {
    scope: defaultScope,
    scopeId: '',
    key: '',
    kind: SecretKind.env,
    value: '',
  }
}

function formFromSecret(secret: SecretMetadataResponse): FormState {
  return {
    scope: secret.scopeType as EditorScope,
    scopeId: secret.scopeId ?? '',
    key: secret.key,
    kind: secret.kind,
    value: '',
  }
}

export function SecretEditorDialog({
  open,
  onOpenChange,
  editSecret,
  defaultScope = 'user',
  teams = [],
  teamsError = null,
  teamsLoading = false,
}: SecretEditorDialogProps) {
  const { isAdmin } = useAuth()
  const isEditing = !!editSecret
  const [form, setForm] = useState<FormState>(() => initialForm(defaultScope))

  const putSecret = usePutSecret()
  const patchSecret = usePatchSecret()
  const isPending = putSecret.isPending || patchSecret.isPending

  useEffect(() => {
    if (!open) {
      return
    }
    setForm(editSecret ? formFromSecret(editSecret) : initialForm(defaultScope))
  }, [open, editSecret, defaultScope])

  const needsScopeId = form.scope === SecretScopeType.team
  const missingValue = !(isEditing || form.value)
  const missingKey = !form.key.trim()
  const missingScopeId = needsScopeId && !form.scopeId.trim()
  const submitDisabled = isPending || missingKey || missingValue || missingScopeId

  const handleSubmit = () => {
    if (missingKey) {
      toast.error('Key is required')
      return
    }
    if (missingScopeId) {
      toast.error('Team is required for team-scoped secrets')
      return
    }
    if (missingValue) {
      toast.error('Value is required for a new secret')
      return
    }

    if (isEditing && editSecret) {
      const request: PatchSecretRequest = {
        ...(form.value ? { value: form.value } : {}),
        ...(form.kind !== editSecret.kind ? { kind: form.kind } : {}),
      }
      patchSecret.mutate(
        { id: editSecret.id, request },
        {
          onSuccess: () => {
            toast.success('Secret updated')
            onOpenChange(false)
          },
          onError: (error) => {
            toast.error('Failed to update secret', { description: getErrorMessage(error) })
          },
        },
      )
      return
    }

    const request: PutSecretRequest = {
      scope: form.scope,
      scopeId: needsScopeId ? form.scopeId : null,
      key: form.key.trim(),
      kind: form.kind,
      value: form.value,
    }
    putSecret.mutate(request, {
      onSuccess: () => {
        toast.success('Secret saved')
        onOpenChange(false)
      },
      onError: (error) => {
        toast.error('Failed to save secret', { description: getErrorMessage(error) })
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>{isEditing ? 'Update Secret' : 'New Secret'}</DialogTitle>
          <DialogDescription>
            {isEditing
              ? 'Replace the stored value. Values are encrypted at rest and never returned by the API.'
              : 'Secrets are encrypted at rest and delivered to the sandbox at task start.'}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="space-y-2">
            <Label>Scope</Label>
            <Select
              value={form.scope}
              onValueChange={(v) =>
                setForm((prev) => ({ ...prev, scope: v as EditorScope, scopeId: '' }))
              }
              disabled={isEditing}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={SecretScopeType.user}>User (only me)</SelectItem>
                <SelectItem value={SecretScopeType.team}>Team</SelectItem>
                <SelectItem value={SecretScopeType.global} disabled={!isAdmin}>
                  Global {isAdmin ? '' : '(admins only)'}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          {needsScopeId && (
            <div className="space-y-2">
              <Label>Team</Label>
              <TeamSelect
                value={form.scopeId}
                onValueChange={(value) => setForm((prev) => ({ ...prev, scopeId: value }))}
                disabled={isEditing || teamsLoading || !!teamsError}
                teams={teams}
                isLoading={teamsLoading}
                error={teamsError}
              />
            </div>
          )}

          <div className="space-y-2">
            <Label>Key</Label>
            <Input
              value={form.key}
              onChange={(e) => setForm((prev) => ({ ...prev, key: e.target.value }))}
              disabled={isEditing}
              placeholder="OPENAI_API_KEY"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="text-xs text-muted-foreground">
              Environment variable name. Case-sensitive. Cannot be changed after creation.
            </p>
          </div>

          <div className="space-y-2">
            <Label>Kind</Label>
            <Select
              value={form.kind}
              onValueChange={(v) => setForm((prev) => ({ ...prev, kind: v as SecretKind }))}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={SecretKind.env}>env (environment variable)</SelectItem>
                <SelectItem value={SecretKind.provider_token}>provider_token</SelectItem>
                <SelectItem value={SecretKind.runtime_file}>runtime_file</SelectItem>
                <SelectItem value={SecretKind.deployment}>deployment</SelectItem>
                <SelectItem value={SecretKind.ssh_key}>ssh_key (SSH private key)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <Label>
              Value
              {isEditing && (
                <span className="ml-2 text-muted-foreground">(leave blank to keep current)</span>
              )}
            </Label>
            {form.kind === SecretKind.ssh_key ? (
              <>
                <Textarea
                  value={form.value}
                  onChange={(e) => setForm((prev) => ({ ...prev, value: e.target.value }))}
                  placeholder={
                    isEditing
                      ? 'Paste a new private key to replace'
                      : '-----BEGIN OPENSSH PRIVATE KEY-----\n…'
                  }
                  className="min-h-[160px] font-mono text-xs"
                  autoComplete="off"
                  spellCheck={false}
                />
                <p className="text-xs text-muted-foreground">
                  PEM/OpenSSH private key. Written to <code>~/.ssh</code> in the sandbox and used
                  for git/ssh over SSH remotes. Never exposed as an environment variable.
                </p>
              </>
            ) : (
              <Input
                type="password"
                value={form.value}
                onChange={(e) => setForm((prev) => ({ ...prev, value: e.target.value }))}
                placeholder={isEditing ? 'Enter new value to replace' : 'Secret value'}
                autoComplete="new-password"
                spellCheck={false}
              />
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={submitDisabled}>
            {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isEditing ? 'Save Changes' : 'Create'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
