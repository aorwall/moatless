import { Link } from '@tanstack/react-router'
import { AlertTriangle, Home, RefreshCw, SearchX } from 'lucide-react'

import { getErrorMessage } from '@/lib/utils/error'
import { Button } from './ui/button'

interface FullPageErrorProps {
  error: unknown
  onRetry?: () => void
  title?: string
  message?: string
}

/** Extract HTTP status from error, handling both instanceof and duck-typing. */
function getHttpStatus(error: unknown): number | null {
  if (
    error != null &&
    typeof error === 'object' &&
    'status' in error &&
    typeof (error as { status: unknown }).status === 'number'
  ) {
    return (error as { status: number }).status
  }
  return null
}

function getErrorInfo(error: unknown): { status: number | null; title: string; message: string } {
  const status = getHttpStatus(error)

  if (status === 404) {
    return {
      status: 404,
      title: 'Not found',
      message: "The resource you're looking for doesn't exist or has been removed.",
    }
  }
  if (status === 403) {
    return {
      status: 403,
      title: 'Access denied',
      message: "You don't have permission to view this resource.",
    }
  }
  if (status !== null && status >= 500) {
    return {
      status,
      title: 'Server error',
      message: 'Something went wrong on the server. Please try again later.',
    }
  }
  if (status !== null) {
    return {
      status,
      title: 'Request failed',
      message: getErrorMessage(error),
    }
  }

  return {
    status: null,
    title: 'Something went wrong',
    message: getErrorMessage(error),
  }
}

export function FullPageError({ error, onRetry, title, message }: FullPageErrorProps) {
  const info = getErrorInfo(error)
  const displayTitle = title ?? info.title
  const displayMessage = message ?? info.message
  const is404 = info.status === 404
  const isRetryable = info.status === null || info.status >= 500

  return (
    <div className="flex h-full items-center justify-center p-8">
      <div className="flex flex-col items-center text-center max-w-md space-y-4">
        <div className="rounded-full bg-muted p-4">
          {is404 ? (
            <SearchX className="h-8 w-8 text-muted-foreground" />
          ) : (
            <AlertTriangle className="h-8 w-8 text-muted-foreground" />
          )}
        </div>

        {info.status && (
          <p className="text-4xl font-bold text-muted-foreground/50">{info.status}</p>
        )}

        <div className="space-y-1">
          <h2 className="text-xl font-semibold">{displayTitle}</h2>
          <p className="text-sm text-muted-foreground">{displayMessage}</p>
        </div>

        <div className="flex items-center gap-3 pt-2">
          <Button variant="outline" size="sm" asChild>
            <Link to="/">
              <Home className="mr-2 h-4 w-4" />
              Dashboard
            </Link>
          </Button>
          {onRetry && isRetryable && (
            <Button variant="outline" size="sm" onClick={onRetry}>
              <RefreshCw className="mr-2 h-4 w-4" />
              Try again
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
