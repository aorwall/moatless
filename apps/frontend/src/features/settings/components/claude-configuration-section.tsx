import { ExternalLink, Loader2, LogOut, Plug } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { CLAUDE_TOKEN_SECRET_KEY, useClaudeToken } from '@/features/settings/hooks/use-claude-token'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'

export function ClaudeConfigurationSection() {
  const { claudeSecret, isLoading, error, saveToken, removeToken, isSaving, isRemoving } =
    useClaudeToken()
  const [token, setToken] = useState('')

  const handleSave = () => {
    saveToken(token, {
      onSuccess: () => {
        toast.success('Claude token saved')
        setToken('')
      },
    })
  }

  const handleRemove = () => {
    removeToken({
      onSuccess: () => {
        toast.success('Claude token removed')
        setToken('')
      },
    })
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="h-8 w-8 animate-spin" data-testid="loading-spinner" />
      </div>
    )
  }

  if (error) {
    return (
      <Alert variant="destructive">
        <AlertTitle>Error</AlertTitle>
        <AlertDescription>Failed to load Claude settings. Please try again later.</AlertDescription>
      </Alert>
    )
  }

  return (
    <ConfigurationSectionWrapper>
      <ConfigurationGroup title="Claude Configuration" showSeparator={false}>
        <p className="text-sm text-muted-foreground mb-4">
          Store a Claude token to enable Claude Code integration
        </p>

        {claudeSecret ? (
          <>
            <Alert className="mb-4">
              <Plug className="h-4 w-4" />
              <AlertTitle>Claude token configured</AlertTitle>
              <AlertDescription className="space-y-2">
                <p className="text-sm">
                  Your token is stored as the{' '}
                  <code className="bg-muted px-1 py-0.5 rounded text-xs">
                    {CLAUDE_TOKEN_SECRET_KEY}
                  </code>{' '}
                  secret and passed to the sandbox at task start.
                </p>
                <p className="text-sm text-muted-foreground">
                  Last updated {new Date(claudeSecret.updatedAt).toLocaleDateString()}
                </p>
              </AlertDescription>
            </Alert>

            <div className="flex justify-end pt-4">
              <Button onClick={handleRemove} disabled={isRemoving} variant="outline" size="sm">
                {isRemoving ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <LogOut className="mr-2 h-4 w-4" />
                )}
                Remove token
              </Button>
            </div>
          </>
        ) : (
          <div className="space-y-4">
            <ConfigurationItem
              label="Claude Token"
              description="Enter your Claude token to authenticate Claude Code in the sandbox"
            >
              <div className="flex gap-2">
                <Input
                  type="password"
                  placeholder="Enter your Claude token"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  className="flex-1 font-mono"
                />
                <Button onClick={handleSave} disabled={isSaving || !token.trim()} variant="outline">
                  {isSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Save'}
                </Button>
              </div>
            </ConfigurationItem>

            <Alert>
              <AlertTitle>How to get a Claude token</AlertTitle>
              <AlertDescription className="space-y-2">
                <p className="text-sm">
                  Run{' '}
                  <code className="bg-muted px-1 py-0.5 rounded text-xs">claude setup-token</code>,
                  follow the authentication flow, and copy the token from the command output.
                </p>
                <p className="text-sm mt-2">
                  For more information, see the{' '}
                  <a
                    href="https://docs.anthropic.com/claude/docs/claude-code"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-blue-600 hover:underline inline-flex items-center gap-1"
                  >
                    Claude Code documentation
                    <ExternalLink className="h-3 w-3" />
                  </a>
                </p>
              </AlertDescription>
            </Alert>
          </div>
        )}
      </ConfigurationGroup>
    </ConfigurationSectionWrapper>
  )
}
