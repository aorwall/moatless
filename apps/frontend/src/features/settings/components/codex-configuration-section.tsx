import { AlertTriangle, ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { CodexDeviceLoginDialog } from '@/features/settings/components/codex-device-login-dialog'
import {
  useCodexConfig,
  useDisconnectCodex,
  useSaveCodexAuthJson,
} from '@/features/settings/hooks/use-codex-connection'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { Textarea } from '@/lib/components/ui/textarea'
import { getErrorMessage } from '@/lib/utils/error'

export function CodexConfigurationSection() {
  const { data: codexConfig, isLoading, error } = useCodexConfig()
  const disconnect = useDisconnectCodex()
  const savePasted = useSaveCodexAuthJson()
  const [loginOpen, setLoginOpen] = useState(false)
  const [authJson, setAuthJson] = useState('')

  const handleDisconnect = () => {
    disconnect.mutate(undefined, {
      onSuccess: () => toast.success('Codex disconnected'),
      onError: (err) =>
        toast.error('Failed to disconnect Codex', {
          description: getErrorMessage(err),
        }),
    })
  }

  const handleSavePasted = () => {
    if (!authJson.trim()) {
      toast.error('Please paste your Codex auth.json')
      return
    }
    savePasted.mutate(
      { authJson },
      {
        onSuccess: () => {
          toast.success('Codex auth.json saved')
          setAuthJson('')
        },
        onError: (err) =>
          toast.error('Failed to save Codex auth.json', {
            description: getErrorMessage(err),
          }),
      },
    )
  }

  if (isLoading) {
    return (
      <div className="flex h-32 items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load Codex settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  const isConnected = codexConfig?.connected ?? false
  const needsReconnect = codexConfig?.needsReconnect ?? false

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Codex Configuration" showSeparator={false}>
        <p className="mb-4 text-sm text-muted-foreground">
          Sign in with your ChatGPT account to run Codex inside your sandboxes.
        </p>

        {isConnected && needsReconnect ? (
          <Alert className="mb-4 border-amber-500/60 bg-amber-500/10 text-amber-700 dark:text-amber-400 [&>svg]:text-amber-700 dark:[&>svg]:text-amber-400">
            <AlertTriangle className="h-4 w-4" />
            <AlertTitle>Reconnect needed</AlertTitle>
            <AlertDescription className="space-y-2">
              <p className="text-sm">
                Your Codex sign-in has expired and could not be refreshed. New Codex tasks will fail
                until you sign in again.
              </p>
              <Button size="sm" onClick={() => setLoginOpen(true)}>
                <ExternalLink className="mr-2 h-4 w-4" />
                Sign in with ChatGPT
              </Button>
            </AlertDescription>
          </Alert>
        ) : isConnected ? (
          <Alert className="mb-4">
            <Plug className="h-4 w-4" />
            <AlertTitle>Connected to ChatGPT</AlertTitle>
            <AlertDescription className="space-y-1">
              {codexConfig?.email && <p className="text-sm">{codexConfig.email}</p>}
              {codexConfig?.planType && (
                <p className="text-sm text-muted-foreground">Plan: {codexConfig.planType}</p>
              )}
              {!(codexConfig?.email || codexConfig?.planType) && (
                <p className="text-sm text-muted-foreground">
                  Authenticated via{' '}
                  {codexConfig?.authMethod === 'device_code'
                    ? 'ChatGPT sign-in'
                    : 'pasted auth.json'}
                  .
                </p>
              )}
            </AlertDescription>
          </Alert>
        ) : (
          <ConfigurationItem
            label="Sign in with ChatGPT"
            description="Authorize Moatless Vibe to use your ChatGPT account."
          >
            <Button onClick={() => setLoginOpen(true)}>
              <ExternalLink className="mr-2 h-4 w-4" />
              Sign in with ChatGPT
            </Button>
          </ConfigurationItem>
        )}

        <Collapsible className="mt-6">
          <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-4 hover:underline">
            Advanced: paste auth.json
          </CollapsibleTrigger>
          <CollapsibleContent className="mt-2 space-y-2">
            <Textarea
              placeholder='{"auth_mode":"chatgpt","tokens":{...}}'
              value={authJson}
              onChange={(e) => setAuthJson(e.target.value)}
              className="font-mono text-xs"
              rows={6}
            />
            <Button
              variant="outline"
              size="sm"
              onClick={handleSavePasted}
              disabled={savePasted.isPending || !authJson.trim()}
            >
              {savePasted.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Save auth.json
            </Button>
          </CollapsibleContent>
        </Collapsible>

        {isConnected && (
          <div className="flex justify-end pt-4">
            <Button
              variant="outline"
              size="sm"
              onClick={handleDisconnect}
              disabled={disconnect.isPending}
            >
              {disconnect.isPending ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : (
                <LogOut className="mr-2 h-4 w-4" />
              )}
              Disconnect
            </Button>
          </div>
        )}
      </ConfigurationGroup>

      <CodexDeviceLoginDialog
        key={loginOpen ? 'open' : 'closed'}
        open={loginOpen}
        onOpenChange={setLoginOpen}
      />
    </ConfigurationSectionWrapper>
  )
}
