import type { GitHubAppOption } from '@moatless/api-client/sandbox/model'
import { Bot, KeyRound, Loader2, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  useAdminGitHubApps,
  useCreateAdminGitHubBotUser,
  useRegisterAdminGitHubApp,
  useRemoveAdminGitHubApp,
  useRotateAdminGitHubAppKey,
} from '@/features/settings/hooks/use-admin-github-connection'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/lib/components/ui/dialog'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

function RegisterAppDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (o: boolean) => void
}) {
  const register = useRegisterAdminGitHubApp()
  const createBotUser = useCreateAdminGitHubBotUser()
  const [githubAppKey, setGithubAppKey] = useState('')
  const [appId, setAppId] = useState('')
  const [privateKeyPem, setPrivateKeyPem] = useState('')

  const reset = () => {
    setGithubAppKey('')
    setAppId('')
    setPrivateKeyPem('')
  }

  const handleRegister = () => {
    const parsedAppId = Number(appId)
    if (!(githubAppKey && privateKeyPem && Number.isSafeInteger(parsedAppId))) {
      return
    }
    register.mutate(
      { githubAppKey, appId: parsedAppId, privateKeyPem },
      {
        onSuccess: (app) => {
          reset()
          onOpenChange(false)
          // Registering makes the App usable; staffing it is the separate
          // second act, offered here rather than assumed.
          toast.success(`Registered ${app.name}`, {
            description: `Create its bot user ${app.botLogin} so work can be attributed to it.`,
            action: {
              label: 'Create bot user',
              onClick: () =>
                createBotUser.mutate(app.githubAppKey, {
                  onSuccess: (user) => toast.success(`Bot user ${user.login} created`),
                  onError: (error) =>
                    toast.error('Failed to create bot user', {
                      description: getErrorMessage(error),
                    }),
                }),
            },
          })
        },
        onError: (error) =>
          toast.error('Failed to register GitHub App', { description: getErrorMessage(error) }),
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Register a GitHub App</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            Create the App at GitHub first, then paste its numeric App ID and signing key here. The
            key is used against GitHub before anything is stored, and can never be read back out.
          </p>
          <div className="space-y-2">
            <Label htmlFor="github-app-key">Name</Label>
            <Input
              id="github-app-key"
              placeholder="review-bot"
              value={githubAppKey}
              onChange={(e) => setGithubAppKey(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              Letters, digits, <code>-</code> and <code>_</code>. This is what bot users and
              repository bindings refer to, so it cannot be changed later.
            </p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="github-app-id">App ID</Label>
            <Input
              id="github-app-id"
              inputMode="numeric"
              placeholder="123456"
              value={appId}
              onChange={(e) => setAppId(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              From the App's own settings page, under <strong>App ID</strong>. Not the installation
              ID in the URL of the page where you installed it, and not the client ID.
            </p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="github-app-pem">Signing key (PEM)</Label>
            <Textarea
              id="github-app-pem"
              className="font-mono text-xs"
              rows={6}
              placeholder="-----BEGIN RSA PRIVATE KEY-----"
              value={privateKeyPem}
              onChange={(e) => setPrivateKeyPem(e.target.value)}
            />
          </div>
          {register.error && <ErrorAlert error={register.error} />}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              onClick={handleRegister}
              disabled={register.isPending || !(githubAppKey && appId && privateKeyPem)}
            >
              {register.isPending && <Loader2 className="animate-spin" />}
              Register
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function RotateKeyDialog({ app, onClose }: { app: GitHubAppOption | null; onClose: () => void }) {
  const rotate = useRotateAdminGitHubAppKey()
  const [privateKeyPem, setPrivateKeyPem] = useState('')

  const handleRotate = () => {
    if (!(app && privateKeyPem)) {
      return
    }
    rotate.mutate(
      { githubAppKey: app.githubAppKey, privateKeyPem },
      {
        onSuccess: (updated) => {
          setPrivateKeyPem('')
          onClose()
          toast.success(`Signing key rotated for ${updated.name}`, {
            description: `New key ${updated.keyFingerprint}. It is in use everywhere from the next call.`,
          })
        },
        onError: (error) =>
          toast.error('Failed to rotate the signing key', {
            description: getErrorMessage(error),
          }),
      },
    )
  }

  return (
    <Dialog open={app !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Rotate the signing key for {app?.githubAppKey}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            The new key is proved against GitHub before the stored one is replaced. If it is
            rejected, this App is left exactly as it is.
          </p>
          <div className="space-y-2">
            <Label htmlFor="github-app-rotate-pem">New signing key (PEM)</Label>
            <Textarea
              id="github-app-rotate-pem"
              className="font-mono text-xs"
              rows={6}
              placeholder="-----BEGIN RSA PRIVATE KEY-----"
              value={privateKeyPem}
              onChange={(e) => setPrivateKeyPem(e.target.value)}
            />
          </div>
          {rotate.error && <ErrorAlert error={rotate.error} />}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="button"
              onClick={handleRotate}
              disabled={rotate.isPending || !privateKeyPem}
            >
              {rotate.isPending && <Loader2 className="animate-spin" />}
              Rotate key
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function AppRow({
  app,
  onRotate,
}: {
  app: GitHubAppOption
  onRotate: (app: GitHubAppOption) => void
}) {
  const removeApp = useRemoveAdminGitHubApp()
  const createBotUser = useCreateAdminGitHubBotUser()

  const handleRemove = () => {
    removeApp.mutate(app.githubAppKey, {
      onSuccess: (result) =>
        toast.success(`Removed ${app.githubAppKey}`, {
          description:
            result.orphanedBotUsers.length > 0
              ? `${result.orphanedBotUsers.length} bot user(s) still name it and can no longer obtain a token.`
              : undefined,
        }),
      onError: (error) =>
        toast.error('Failed to remove the App', { description: getErrorMessage(error) }),
    })
  }

  const handleCreateBotUser = () => {
    createBotUser.mutate(app.githubAppKey, {
      onSuccess: (user) => toast.success(`Bot user ${user.login} created`),
      onError: (error) =>
        toast.error('Failed to create bot user', { description: getErrorMessage(error) }),
    })
  }

  return (
    <div className="flex flex-wrap items-start justify-between gap-3 rounded-md border p-3">
      <div className="space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{app.name}</span>
          <code className="text-xs text-muted-foreground">{app.githubAppKey}</code>
          {app.deploymentConfigured && <Badge variant="secondary">Configured by deployment</Badge>}
        </div>
        <p className="text-xs text-muted-foreground">
          {app.keyFingerprint ? `Signing key ${app.keyFingerprint} · ` : ''}
          {app.installations.length} account{app.installations.length === 1 ? '' : 's'}
          {app.botUserId ? ` · bot ${app.botLogin}` : ' · no bot user'}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        {!app.botUserId && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={handleCreateBotUser}
            disabled={createBotUser.isPending}
          >
            {createBotUser.isPending ? <Loader2 className="animate-spin" /> : <Bot />}
            Create bot user
          </Button>
        )}
        {/* The deployment-configured App is shown so admins can see it, and is
            not editable here — nothing done on this page can make sign-in
            unreachable. */}
        {!app.deploymentConfigured && (
          <>
            <Button type="button" size="sm" variant="outline" onClick={() => onRotate(app)}>
              <KeyRound />
              Rotate key
            </Button>
            <Button
              type="button"
              size="sm"
              variant="destructive"
              onClick={handleRemove}
              disabled={removeApp.isPending}
            >
              {removeApp.isPending ? <Loader2 className="animate-spin" /> : <Trash2 />}
              Remove
            </Button>
          </>
        )}
      </div>
    </div>
  )
}

export function GitHubAppsCard() {
  const appsQuery = useAdminGitHubApps()
  const [registerOpen, setRegisterOpen] = useState(false)
  const [rotating, setRotating] = useState<GitHubAppOption | null>(null)

  return (
    <Card>
      <CardHeader>
        <CardTitle>GitHub Apps</CardTitle>
        <CardDescription>
          Register an App so repositories can be reached through it and work can run as its bot.
          Changes apply everywhere from the next call — no restart, no deploy.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {appsQuery.isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading GitHub Apps…
          </div>
        ) : (
          <>
            {appsQuery.error && <ErrorAlert error={appsQuery.error} />}
            {appsQuery.data?.apps.length === 0 && (
              <p className="text-sm text-muted-foreground">No GitHub Apps are registered yet.</p>
            )}
            {appsQuery.data?.apps.map((app) => (
              <AppRow key={app.githubAppKey} app={app} onRotate={setRotating} />
            ))}
            <Button type="button" onClick={() => setRegisterOpen(true)}>
              <Plus />
              Register App
            </Button>
          </>
        )}
      </CardContent>
      <RegisterAppDialog open={registerOpen} onOpenChange={setRegisterOpen} />
      <RotateKeyDialog app={rotating} onClose={() => setRotating(null)} />
    </Card>
  )
}
