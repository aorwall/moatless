import { AlertCircle, Loader2, XCircle } from 'lucide-react'
import { ApiError } from '@/lib/api/client'
import { Button } from '@/lib/components/ui/button'
import type { SandboxEvent } from '../types'
import { RecentEvents } from './recent-events'

type EnvironmentStatus = 'not_created' | 'initializing' | 'ready' | 'stopped' | 'removing' | 'error'

interface SandboxStatusConfig {
  icon: React.ReactNode
  title: string
  message: string
  showRetry: boolean
}

function getSandboxStatusConfig(status: EnvironmentStatus, error?: string): SandboxStatusConfig {
  const configs: Record<EnvironmentStatus, SandboxStatusConfig> = {
    not_created: {
      icon: <AlertCircle className="h-12 w-12 text-orange-500" />,
      title: 'Sandbox Not Running',
      message: "Environment hasn't been created yet",
      showRetry: true,
    },
    initializing: {
      icon: <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />,
      title: 'Setting Up Sandbox',
      message: 'Please wait while the workspace is being prepared...',
      showRetry: false,
    },
    stopped: {
      icon: <AlertCircle className="h-12 w-12 text-orange-500" />,
      title: 'Sandbox Stopped',
      message: 'The workspace container was stopped',
      showRetry: true,
    },
    error: {
      icon: <XCircle className="h-12 w-12 text-destructive" />,
      title: 'Sandbox Error',
      message: error || 'An error occurred with the workspace environment',
      showRetry: true,
    },
    removing: {
      icon: <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />,
      title: 'Removing Sandbox',
      message: 'Please wait while the workspace is being removed...',
      showRetry: false,
    },
    ready: {
      icon: null,
      title: '',
      message: '',
      showRetry: false,
    },
  }
  return configs[status]
}

function getMutationErrorMessage(error: Error): string {
  if (error instanceof ApiError) {
    const data = error.data as { error?: unknown } | undefined
    if (typeof data?.error === 'string') {
      return data.error
    }
  }
  return error.message
}

interface SandboxStatusOverlayProps {
  environmentStatus: EnvironmentStatus
  environmentError?: string
  sandboxEvents?: SandboxEvent[]
  onRetry: () => void
  isRetrying: boolean
  retryError?: Error | null
}

export function SandboxStatusOverlay({
  environmentStatus,
  environmentError,
  sandboxEvents,
  onRetry,
  isRetrying,
  retryError,
}: SandboxStatusOverlayProps) {
  if (environmentStatus === 'ready') {
    return null
  }

  const config = getSandboxStatusConfig(environmentStatus, environmentError)

  return (
    <div className="absolute inset-0 flex items-center justify-center z-10 bg-background/80 backdrop-blur-sm">
      <div className="flex flex-col items-center justify-center text-center space-y-4 p-8">
        {config.icon}
        <h3 className="text-lg font-semibold">{config.title}</h3>
        <p className="text-sm text-muted-foreground max-w-md">{config.message}</p>
        {environmentStatus === 'initializing' && sandboxEvents && sandboxEvents.length > 0 && (
          <RecentEvents events={sandboxEvents} />
        )}
        {config.showRetry && (
          <>
            <Button variant="outline" size="sm" onClick={onRetry} disabled={isRetrying}>
              {isRetrying ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Starting...
                </>
              ) : (
                'Start Sandbox'
              )}
            </Button>
            {retryError && (
              <p className="text-destructive text-xs max-w-md line-clamp-3">
                {getMutationErrorMessage(retryError)}
              </p>
            )}
          </>
        )}
      </div>
    </div>
  )
}
