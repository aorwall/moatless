import { Loader2 } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

/** Sandbox provisioning-failure banner with a retry button. */
export function SandboxErrorBanner({
  errorMessage,
  onRetry,
  isRetrying,
}: {
  errorMessage: string
  onRetry: () => void
  isRetrying: boolean
}) {
  return (
    <div className="mx-4 mb-2 px-4 py-3 bg-destructive/10 border border-destructive/20 rounded-lg flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <p className="text-sm text-destructive font-medium">Sandbox setup failed</p>
        <p className="text-sm text-muted-foreground truncate">{errorMessage}</p>
      </div>
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
    </div>
  )
}
