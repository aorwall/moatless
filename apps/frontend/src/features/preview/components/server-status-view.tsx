import { AlertCircle, Loader2, XCircle } from 'lucide-react'

/** Status placeholder shown while the active server is not yet serving. */
export function ServerStatusView({
  server,
}: {
  server: {
    status: string
    label: string
    name: string
    error?: string | null
    detail?: string | null
  }
}) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-center space-y-4 p-8">
      {(server.status === 'starting' || server.status === 'installing') && (
        <>
          <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />
          <h3 className="text-lg font-semibold">
            {server.status === 'installing' ? 'Installing Dependencies' : 'Server Starting'}
          </h3>
          <p className="text-sm text-muted-foreground max-w-md">
            Please wait while {server.label || 'the server'} starts...
          </p>
        </>
      )}
      {server.status === 'failed' && (
        <>
          <XCircle className="h-12 w-12 text-destructive" />
          <h3 className="text-lg font-semibold">Server Failed</h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {server.error || 'The server encountered an error'}
          </p>
        </>
      )}
      {server.status === 'disabled' && (
        <>
          <AlertCircle className="h-12 w-12 text-muted-foreground" />
          <h3 className="text-lg font-semibold">Server Not Started</h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {server.detail ||
              `${server.label || 'The server'} is declared by the workspace but this task is headless, so it was never started`}
          </p>
        </>
      )}
      {server.status === 'stopped' && (
        <>
          <AlertCircle className="h-12 w-12 text-orange-500" />
          <h3 className="text-lg font-semibold">Server Stopped</h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {server.label || 'The server'} is not currently running
          </p>
        </>
      )}
    </div>
  )
}
