import { AlertCircle, Loader2, XCircle } from 'lucide-react'
import { ApiError } from '@/lib/api/client'
import { Button } from '@/lib/components/ui/button'

type SandboxStatus =
  | 'not_created'
  | 'initializing'
  | 'ready'
  | 'stopped'
  | 'removing'
  | 'removed'
  | 'error'

interface SandboxStatusConfig {
  icon: React.ReactNode
  title: string
  message: string
  showRetry: boolean
}

function getSandboxStatusConfig(
  status: SandboxStatus,
  error?: string | null | undefined,
): SandboxStatusConfig {
  const configs: Record<SandboxStatus, SandboxStatusConfig> = {
    not_created: {
      icon: <AlertCircle className="h-12 w-12 text-orange-500" />,
      title: 'Sandbox Not Running',
      message: "Environment hasn't been created yet",
      showRetry: true,
    },
    initializing: {
      icon: <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />,
      title: 'Setting Up Sandbox',
      message: 'Please wait while the sandbox is being prepared...',
      showRetry: false,
    },
    stopped: {
      icon: <AlertCircle className="h-12 w-12 text-orange-500" />,
      title: 'Sandbox Stopped',
      message: 'The sandbox was stopped',
      showRetry: true,
    },
    error: {
      icon: <XCircle className="h-12 w-12 text-destructive" />,
      title: 'Sandbox Error',
      message: error || 'An error occurred with the sandbox environment',
      showRetry: true,
    },
    removing: {
      icon: <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />,
      title: 'Removing Sandbox',
      message: 'Please wait while the sandbox is being removed...',
      showRetry: false,
    },
    removed: {
      icon: <AlertCircle className="h-12 w-12 text-muted-foreground" />,
      title: 'Sandbox Removed',
      message: 'The sandbox environment has been removed',
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
  // ApiError from apiClient
  if (error instanceof ApiError) {
    const data = error.data as { error?: unknown } | undefined
    if (typeof data?.error === 'string') {
      return data.error
    }
  }
  // ErrorType from generated api-client (custom-instance)
  const dataError = (error as { data?: { error?: unknown } }).data
  if (typeof dataError?.error === 'string') {
    return dataError.error
  }
  return error.message
}

interface SandboxStatusOverlayProps {
  sandboxStatus: SandboxStatus
  sandboxError?: string | null | undefined
  onRetry: () => void
  isRetrying: boolean
  retryError?: Error | null | undefined
}

export function SandboxStatusOverlay({
  sandboxStatus,
  sandboxError,
  onRetry,
  isRetrying,
  retryError,
}: SandboxStatusOverlayProps) {
  if (sandboxStatus === 'ready') {
    return null
  }

  const config = getSandboxStatusConfig(sandboxStatus, sandboxError)

  const showProvisioningError = config.showRetry && retryError

  return (
    <div className="absolute inset-0 flex items-center justify-center z-10 bg-background/80 backdrop-blur-sm">
      <div className="flex flex-col items-center justify-center text-center space-y-4 p-8">
        {showProvisioningError ? (
          <>
            <XCircle className="h-12 w-12 text-destructive" />
            <h3 className="text-lg font-semibold">Provisioning Failed</h3>
            <p className="text-sm text-destructive max-w-md">
              {getMutationErrorMessage(retryError)}
            </p>
            <Button variant="outline" size="sm" onClick={onRetry} disabled={isRetrying}>
              {isRetrying ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Retrying...
                </>
              ) : (
                'Retry'
              )}
            </Button>
          </>
        ) : (
          <>
            {config.icon}
            <h3 className="text-lg font-semibold">{config.title}</h3>
            <p className="text-sm text-muted-foreground max-w-md">{config.message}</p>
            {config.showRetry && (
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
            )}
          </>
        )}
      </div>
    </div>
  )
}
