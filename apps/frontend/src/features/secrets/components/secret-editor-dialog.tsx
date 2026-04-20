import { SecretKind, type SecretMetadataResponse, SecretScopeType } from '@moatless/api-client'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  type PatchSecretRequest,
  type PutSecretRequest,
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
import { getErrorMessage } from '@/lib/utils/error'

type EditorScope = 'user' | 'team' | 'global'

interface SecretEditorDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  editSecret?: SecretMetadataResponse | null
  defaultScope?: EditorScope
}

interface FormState {
  scope: EditorScope
  scopeId: string
  key: string
  target: string
  kind: SecretKind
  value: string
}

const TARGET_OPTIONS = [
  { value: 'agent', label: 'agent (Claude/tasks)' },
  { value: 'all_servers', label: 'all_servers' },
]

function initialForm(defaultScope: EditorScope): FormState {
  return {
    scope: defaultScope,
    scopeId: '',
    key: '',
    target: 'agent',
    kind: SecretKind.env,
    value: '',
  }
}

function formFromSecret(secret: SecretMetadataResponse): FormState {
  return {
    scope: secret.scopeType as EditorScope,
    scopeId: secret.scopeId ?? '',
    key: secret.key,
    target: secret.target,
    kind: secret.kind,
    value: '',
  }
}

export function SecretEditorDialog({
  open,
  onOpenChange,
  editSecret,
  defaultScope = 'user',
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
      toast.error('Team ID is required for team-scoped secrets')
      return
    }
    if (missingValue) {
      toast.error('Value is required for a new secret')
      return
    }

    if (isEditing && editSecret) {
      const request: PatchSecretRequest = {
        value: form.value ? form.value : undefined,
        target: form.target !== editSecret.target ? form.target : undefined,
        kind: form.kind !== editSecret.kind ? form.kind : undefined,
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
      target: form.target,
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
              ? 'Replace the stored value or change the delivery target. Values are encrypted at rest and never returned by the API.'
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
              <Label>Team ID</Label>
              <Input
                value={form.scopeId}
                onChange={(e) => setForm((prev) => ({ ...prev, scopeId: e.target.value }))}
                disabled={isEditing}
                placeholder="team-…"
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
            <Label>Target</Label>
            <Select
              value={form.target}
              onValueChange={(v) => setForm((prev) => ({ ...prev, target: v }))}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TARGET_OPTIONS.map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>
                    {opt.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              Which container receives this secret. Use <code>agent</code> for Claude runtime.
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
            <Input
              type="password"
              value={form.value}
              onChange={(e) => setForm((prev) => ({ ...prev, value: e.target.value }))}
              placeholder={isEditing ? 'Enter new value to replace' : 'Secret value'}
              autoComplete="new-password"
              spellCheck={false}
            />
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
