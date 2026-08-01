import type { AdapterAppSummary, SaveAdapterAppRequest } from '@moatless/api-client'
import { Link } from '@tanstack/react-router'
import { AppWindow, ExternalLink, Loader2, Pencil, Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  useAdapterApps,
  useDeleteAdapterApp,
  useSaveAdapterApp,
} from '@/features/integrations/hooks/use-adapter-apps'
import { getAdapterLabel } from '@/features/integrations/utils/adapter-display'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/lib/components/ui/alert-dialog'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/lib/components/ui/select'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/lib/components/ui/table'
import { getErrorMessage } from '@/lib/utils/error'

type EditableAdapterKind = 'slack' | 'telegram' | 'linear'
type AdapterKind = EditableAdapterKind | 'github'

const editableAdapterKinds: EditableAdapterKind[] = ['slack', 'telegram', 'linear']

const requiredSecrets: Record<AdapterKind, string[]> = {
  slack: ['bot_token', 'signing_secret'],
  telegram: ['bot_token'],
  linear: ['oauth_client_id', 'oauth_client_secret'],
  github: ['private_key'],
}

function isEditableAdapterKind(value: string): value is EditableAdapterKind {
  return editableAdapterKinds.includes(value as EditableAdapterKind)
}

function configRecord(config: unknown): Record<string, unknown> {
  return typeof config === 'object' && config !== null ? (config as Record<string, unknown>) : {}
}

function configString(config: unknown, key: string): string {
  const value = configRecord(config)[key]
  return typeof value === 'string' || typeof value === 'number' ? String(value) : ''
}

function formatTimestamp(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function adapterStatus(app: AdapterAppSummary): 'configured' | 'missing-secret' {
  const names = requiredSecrets[app.adapterKind as AdapterKind] ?? []
  return names.every((name) => Boolean(app.secretFingerprints[name]))
    ? 'configured'
    : 'missing-secret'
}

function statusBadge(app: AdapterAppSummary) {
  const status = adapterStatus(app)
  if (status === 'configured') {
    return <Badge variant="secondary">Configured</Badge>
  }
  return <Badge variant="destructive">Missing secrets</Badge>
}

function secretSummary(app: AdapterAppSummary): string {
  const entries = Object.entries(app.secretFingerprints)
  if (entries.length === 0) {
    return 'No stored secrets'
  }
  return entries.map(([name, fingerprint]) => `${name}: ${fingerprint}`).join(', ')
}

function configSummary(app: AdapterAppSummary): string {
  switch (app.adapterKind) {
    case 'slack': {
      const botUserId = configString(app.config, 'bot_user_id')
      const teamName = configString(app.config, 'team_name')
      return (
        [teamName, botUserId && `bot ${botUserId}`].filter(Boolean).join(' - ') || 'Default app'
      )
    }
    case 'telegram': {
      const mode = configString(app.config, 'mode') || 'webhook'
      const username = configString(app.config, 'bot_username')
      return [mode, username && `@${username}`].filter(Boolean).join(' - ')
    }
    case 'linear': {
      const callbackUrl = configString(app.config, 'oauth_callback_url')
      const apiBaseUrl = configString(app.config, 'api_base_url')
      return [callbackUrl, apiBaseUrl].filter(Boolean).join(' - ') || 'Default client'
    }
    case 'github': {
      const appId = configString(app.config, 'app_id')
      const botLogin = configString(app.config, 'bot_login')
      return [appId && `App ${appId}`, botLogin].filter(Boolean).join(' - ') || 'GitHub App'
    }
    default:
      return 'Adapter app'
  }
}

function blankToNull(value: string): string | null {
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

interface AdapterAppFormDialogProps {
  adapterKind: EditableAdapterKind
  app?: AdapterAppSummary | undefined
  open: boolean
  onOpenChange: (open: boolean) => void
}

function AdapterAppFormDialog({ adapterKind, app, open, onOpenChange }: AdapterAppFormDialogProps) {
  const save = useSaveAdapterApp()
  const [botToken, setBotToken] = useState('')
  const [signingSecret, setSigningSecret] = useState('')
  const [botUserId, setBotUserId] = useState('')
  const [telegramMode, setTelegramMode] = useState('webhook')
  const [webhookSecret, setWebhookSecret] = useState('')
  const [oauthClientId, setOauthClientId] = useState('')
  const [oauthClientSecret, setOauthClientSecret] = useState('')
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState('')
  const [apiBaseUrl, setApiBaseUrl] = useState('')

  useEffect(() => {
    if (!open) {
      return
    }
    setBotToken('')
    setSigningSecret('')
    setWebhookSecret('')
    setOauthClientId('')
    setOauthClientSecret('')
    setBotUserId(configString(app?.config, 'bot_user_id'))
    setTelegramMode(configString(app?.config, 'mode') || 'webhook')
    setOauthCallbackUrl(configString(app?.config, 'oauth_callback_url'))
    setApiBaseUrl(configString(app?.config, 'api_base_url'))
  }, [app, open])

  const title = `${app ? 'Edit' : 'Configure'} ${getAdapterLabel(adapterKind)}`
  const canSubmit =
    adapterKind === 'slack'
      ? botToken.trim().length > 0 && signingSecret.trim().length > 0
      : adapterKind === 'telegram'
        ? botToken.trim().length > 0
        : true

  const handleSubmit = () => {
    let data: SaveAdapterAppRequest
    if (adapterKind === 'slack') {
      data = {
        adapterKind: 'slack',
        botToken,
        signingSecret,
        botUserId: blankToNull(botUserId),
      }
    } else if (adapterKind === 'telegram') {
      data = {
        adapterKind: 'telegram',
        botToken,
        mode: telegramMode,
      }
    } else {
      data = {
        adapterKind: 'linear',
        webhookSecret: blankToNull(webhookSecret),
        oauthClientId: blankToNull(oauthClientId),
        oauthClientSecret: blankToNull(oauthClientSecret),
        oauthCallbackUrl: blankToNull(oauthCallbackUrl),
        apiBaseUrl: blankToNull(apiBaseUrl),
      }
    }

    save.mutate(
      { adapterKind, appKey: app?.appKey ?? 'default', data },
      {
        onSuccess: (saved) => {
          toast.success(`${getAdapterLabel(saved.adapterKind)} app saved`)
          onOpenChange(false)
        },
        onError: (error) =>
          toast.error(`Failed to save ${getAdapterLabel(adapterKind)} app`, {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Secrets are write-only. Existing fingerprints stay visible in the Apps list.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {adapterKind === 'slack' && (
            <>
              <div className="space-y-2">
                <Label htmlFor="slack-bot-token">Bot token</Label>
                <Input
                  id="slack-bot-token"
                  type="password"
                  value={botToken}
                  onChange={(event) => setBotToken(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="slack-signing-secret">Signing secret</Label>
                <Input
                  id="slack-signing-secret"
                  type="password"
                  value={signingSecret}
                  onChange={(event) => setSigningSecret(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="slack-bot-user-id">Bot user ID</Label>
                <Input
                  id="slack-bot-user-id"
                  value={botUserId}
                  onChange={(event) => setBotUserId(event.target.value)}
                />
              </div>
            </>
          )}

          {adapterKind === 'telegram' && (
            <>
              <div className="space-y-2">
                <Label htmlFor="telegram-bot-token">Bot token</Label>
                <Input
                  id="telegram-bot-token"
                  type="password"
                  value={botToken}
                  onChange={(event) => setBotToken(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="telegram-mode">Mode</Label>
                <Select value={telegramMode} onValueChange={setTelegramMode}>
                  <SelectTrigger id="telegram-mode">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="webhook">Webhook</SelectItem>
                    <SelectItem value="polling">Polling</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {telegramMode === 'polling' && (
                <p className="text-sm text-muted-foreground">
                  Polling starts or stops after the backend restarts.
                </p>
              )}
            </>
          )}

          {adapterKind === 'linear' && (
            <>
              <div className="space-y-2">
                <Label htmlFor="linear-oauth-client-id">OAuth client ID</Label>
                <Input
                  id="linear-oauth-client-id"
                  type="password"
                  value={oauthClientId}
                  onChange={(event) => setOauthClientId(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="linear-oauth-client-secret">OAuth client secret</Label>
                <Input
                  id="linear-oauth-client-secret"
                  type="password"
                  value={oauthClientSecret}
                  onChange={(event) => setOauthClientSecret(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="linear-webhook-secret">Webhook secret</Label>
                <Input
                  id="linear-webhook-secret"
                  type="password"
                  value={webhookSecret}
                  onChange={(event) => setWebhookSecret(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="linear-oauth-callback-url">OAuth callback URL</Label>
                <Input
                  id="linear-oauth-callback-url"
                  value={oauthCallbackUrl}
                  onChange={(event) => setOauthCallbackUrl(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="linear-api-base-url">API base URL</Label>
                <Input
                  id="linear-api-base-url"
                  value={apiBaseUrl}
                  onChange={(event) => setApiBaseUrl(event.target.value)}
                />
              </div>
            </>
          )}

          {save.error && <ErrorAlert error={save.error} />}
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" onClick={handleSubmit} disabled={save.isPending || !canSubmit}>
            {save.isPending && <Loader2 className="animate-spin" />}
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function DeleteAdapterAppDialog({
  app,
  open,
  onOpenChange,
}: {
  app: AdapterAppSummary | null
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const remove = useDeleteAdapterApp()

  const handleDelete = () => {
    if (!app) {
      return
    }
    remove.mutate(
      { adapterKind: app.adapterKind, appKey: app.appKey },
      {
        onSuccess: () => toast.success(`${getAdapterLabel(app.adapterKind)} app deleted`),
        onError: (error) =>
          toast.error(`Failed to delete ${getAdapterLabel(app.adapterKind)} app`, {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            Delete {app ? getAdapterLabel(app.adapterKind) : 'Adapter'} app?
          </AlertDialogTitle>
          <AlertDialogDescription>
            Runtime lookups for app key {app?.appKey ?? 'default'} will stop resolving after this
            app is deleted.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={handleDelete}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function AdapterAppsTable({
  apps,
  onEdit,
  onDelete,
}: {
  apps: AdapterAppSummary[]
  onEdit: (adapterKind: EditableAdapterKind, app?: AdapterAppSummary) => void
  onDelete: (app: AdapterAppSummary) => void
}) {
  if (apps.length === 0) {
    return (
      <div className="flex min-h-48 flex-col items-center justify-center gap-3 rounded-md border border-dashed text-center">
        <AppWindow className="h-8 w-8 text-muted-foreground" />
        <div>
          <p className="font-medium">No Adapter apps configured</p>
          <p className="text-sm text-muted-foreground">Configure Slack, Telegram, or Linear.</p>
        </div>
      </div>
    )
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Adapter</TableHead>
          <TableHead>App key</TableHead>
          <TableHead>Status</TableHead>
          <TableHead>Config</TableHead>
          <TableHead>Secrets</TableHead>
          <TableHead>Updated</TableHead>
          <TableHead className="text-right">Actions</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {apps.map((app) => {
          const editableAdapterKind = isEditableAdapterKind(app.adapterKind)
            ? app.adapterKind
            : null
          return (
            <TableRow key={`${app.adapterKind}:${app.appKey}`}>
              <TableCell className="font-medium">{getAdapterLabel(app.adapterKind)}</TableCell>
              <TableCell>
                <code className="text-xs text-muted-foreground">{app.appKey}</code>
              </TableCell>
              <TableCell>{statusBadge(app)}</TableCell>
              <TableCell className="max-w-64 whitespace-normal text-muted-foreground">
                {configSummary(app)}
              </TableCell>
              <TableCell className="max-w-72 whitespace-normal font-mono text-xs text-muted-foreground">
                {secretSummary(app)}
              </TableCell>
              <TableCell>{formatTimestamp(app.updatedAt)}</TableCell>
              <TableCell>
                <div className="flex justify-end gap-2">
                  {app.adapterKind === 'github' ? (
                    <Button asChild size="sm" variant="outline">
                      <Link to="/admin/integrations/github">
                        <ExternalLink />
                        Manage
                      </Link>
                    </Button>
                  ) : (
                    <>
                      {editableAdapterKind && (
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => onEdit(editableAdapterKind, app)}
                        >
                          <Pencil />
                          Edit
                        </Button>
                      )}
                      <Button size="sm" variant="destructive" onClick={() => onDelete(app)}>
                        <Trash2 />
                        Delete
                      </Button>
                    </>
                  )}
                </div>
              </TableCell>
            </TableRow>
          )
        })}
      </TableBody>
    </Table>
  )
}

export function AdminAdapterAppsPage() {
  const appsQuery = useAdapterApps()
  const [editing, setEditing] = useState<{
    adapterKind: EditableAdapterKind
    app?: AdapterAppSummary | undefined
  } | null>(null)
  const [deleting, setDeleting] = useState<AdapterAppSummary | null>(null)

  const apps = useMemo(
    () =>
      [...(appsQuery.data ?? [])].sort((left, right) => {
        const adapterCompare = left.adapterKind.localeCompare(right.adapterKind)
        return adapterCompare === 0 ? left.appKey.localeCompare(right.appKey) : adapterCompare
      }),
    [appsQuery.data],
  )

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Adapter Apps</h1>
          <p className="text-sm text-muted-foreground">
            Deployment-wide app credentials used by Adapter connections.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {editableAdapterKinds.map((adapterKind) => (
            <Button key={adapterKind} variant="outline" onClick={() => setEditing({ adapterKind })}>
              <Plus />
              {getAdapterLabel(adapterKind)}
            </Button>
          ))}
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Apps</CardTitle>
          <CardDescription>
            Connections choose an app key; missing app credentials make that Adapter unavailable.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {appsQuery.isLoading ? (
            <div className="flex min-h-48 items-center justify-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading Adapter apps
            </div>
          ) : appsQuery.error ? (
            <ErrorAlert error={appsQuery.error} />
          ) : (
            <AdapterAppsTable
              apps={apps}
              onEdit={(adapterKind, app) => setEditing({ adapterKind, app })}
              onDelete={setDeleting}
            />
          )}
        </CardContent>
      </Card>

      {editing && (
        <AdapterAppFormDialog
          adapterKind={editing.adapterKind}
          app={editing.app}
          open={editing !== null}
          onOpenChange={(open) => !open && setEditing(null)}
        />
      )}
      <DeleteAdapterAppDialog
        app={deleting}
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
      />
    </div>
  )
}
