import { AlertCircle, Server } from 'lucide-react'

interface EnvironmentIndicatorProps {
  status: 'initializing' | 'error'
  errorMessage?: string | undefined
}

export function EnvironmentIndicator({ status, errorMessage }: EnvironmentIndicatorProps) {
  if (status === 'error') {
    return (
      <div className="flex items-center gap-2 text-destructive">
        <AlertCircle className="h-4 w-4" />
        <span className="text-sm">
          Environment setup failed{errorMessage ? `: ${errorMessage}` : ''}
        </span>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-2 text-muted-foreground animate-pulse">
      <Server className="h-4 w-4" />
      <span className="text-sm">Setting up environment...</span>
    </div>
  )
}
