import { AlertTriangle, ExternalLink, X } from 'lucide-react'
import { useState } from 'react'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/lib/components/ui/card'
import { useSystemStatus } from '../hooks/use-settings'

export function SystemStatusWarning() {
  const { data: systemStatus, isLoading, error } = useSystemStatus()
  const [dismissed, setDismissed] = useState(false)

  // Don't show anything while loading or if there's an error
  if (isLoading || error || dismissed) {
    return null
  }

  // Don't show if Claude is properly configured
  if (systemStatus?.claude.isValidBin && systemStatus?.claude.isValidConfig) {
    return null
  }

  const hasClaudeIssues = !systemStatus?.claude.isValidBin || !systemStatus?.claude.isValidConfig
  const hasDockerIssues = !systemStatus?.docker.isRunning

  if (!hasClaudeIssues && !hasDockerIssues) {
    return null
  }

  return (
    <Alert variant="destructive" className="mb-6">
      <AlertTriangle className="h-4 w-4" />
      <AlertDescription className="flex items-center justify-between">
        <div className="flex-1">
          <strong>System Configuration Issues Detected</strong>
          <div className="mt-2 space-y-1">
            {!systemStatus?.claude.isValidBin && (
              <div className="text-sm">
                • Claude binary not found. Please install Claude Code using{' '}
                <code className="px-1 py-0.5 bg-muted rounded text-xs">
                  npm install -g @anthropic-ai/claude-code
                </code>
              </div>
            )}
            {!systemStatus?.claude.isValidConfig && (
              <div className="text-sm">
                • Claude configuration not found. Please run{' '}
                <code className="px-1 py-0.5 bg-muted rounded text-xs">claude auth</code> to
                authenticate.
              </div>
            )}
            {hasDockerIssues && (
              <div className="text-sm">
                • Docker is not running. Please start Docker to enable workspace features.
              </div>
            )}
          </div>
          <Button
            variant="outline"
            size="sm"
            className="mt-3"
            onClick={() =>
              window.open('https://docs.anthropic.com/claude/docs/claude-code', '_blank')
            }
          >
            <ExternalLink className="mr-2 h-3 w-3" />
            View Setup Guide
          </Button>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setDismissed(true)}
          className="ml-4 h-6 w-6 p-0"
        >
          <X className="h-4 w-4" />
        </Button>
      </AlertDescription>
    </Alert>
  )
}

export function SystemStatusCard() {
  const { data: systemStatus, isLoading, error } = useSystemStatus()

  if (isLoading) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>System Status</CardTitle>
          <CardDescription>Checking system configuration...</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="space-y-2">
            <div className="h-4 bg-muted animate-pulse rounded" />
            <div className="h-4 bg-muted animate-pulse rounded w-3/4" />
            <div className="h-4 bg-muted animate-pulse rounded w-1/2" />
          </div>
        </CardContent>
      </Card>
    )
  }

  if (error) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>System Status</CardTitle>
          <CardDescription>Unable to check system status</CardDescription>
        </CardHeader>
        <CardContent>
          <Alert variant="destructive">
            <AlertTriangle className="h-4 w-4" />
            <AlertDescription>
              Failed to retrieve system status. Please try again later.
            </AlertDescription>
          </Alert>
        </CardContent>
      </Card>
    )
  }

  if (!systemStatus) {
    return null
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>System Status</CardTitle>
        <CardDescription>Current configuration status</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
          <div className="space-y-2">
            <h4 className="font-semibold">Claude Code</h4>
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <span className="text-muted-foreground">Binary:</span>
                <div className="flex items-center gap-2 mt-1">
                  <div
                    className={`w-2 h-2 rounded-full ${
                      systemStatus.claude.isValidBin ? 'bg-green-500' : 'bg-red-500'
                    }`}
                  />
                  <span className="font-mono text-xs">
                    {systemStatus.claude.binPath || 'Not found'}
                  </span>
                </div>
              </div>
              <div>
                <span className="text-muted-foreground">Config:</span>
                <div className="flex items-center gap-2 mt-1">
                  <div
                    className={`w-2 h-2 rounded-full ${
                      systemStatus.claude.isValidConfig ? 'bg-green-500' : 'bg-red-500'
                    }`}
                  />
                  <span className="font-mono text-xs">
                    {systemStatus.claude.configPath || 'Not found'}
                  </span>
                </div>
              </div>
            </div>
          </div>

          <div className="space-y-2">
            <h4 className="font-semibold">Docker</h4>
            <div className="flex items-center gap-2 text-sm">
              <div
                className={`w-2 h-2 rounded-full ${
                  systemStatus.docker.isRunning ? 'bg-green-500' : 'bg-red-500'
                }`}
              />
              <span>
                {systemStatus.docker.isRunning ? 'Running' : 'Not running'}
                {systemStatus.docker.version && (
                  <span className="text-muted-foreground ml-2">
                    ({systemStatus.docker.version})
                  </span>
                )}
              </span>
            </div>
          </div>

          {(!systemStatus.claude.isValidBin || !systemStatus.claude.isValidConfig) && (
            <div className="pt-2 border-t">
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  window.open('https://docs.anthropic.com/claude/docs/claude-code', '_blank')
                }
              >
                <ExternalLink className="mr-2 h-3 w-3" />
                Setup Guide
              </Button>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
