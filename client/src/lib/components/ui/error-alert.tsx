import { AlertCircle, X } from 'lucide-react'
import { useApiError } from '@/lib/hooks/use-api-error'
import { Alert, AlertDescription, AlertTitle } from './alert'
import { Button } from './button'

interface ErrorAlertProps {
  error: unknown
  title?: string
  className?: string
  onDismiss?: () => void
  showValidationDetails?: boolean
}

export function ErrorAlert({
  error,
  title,
  className,
  onDismiss,
  showValidationDetails = true,
}: ErrorAlertProps) {
  const { formatErrorForDisplay } = useApiError()

  // Get formatted error details
  const errorDetails = formatErrorForDisplay(error)
  const displayTitle = title || errorDetails.title

  return (
    <Alert variant="destructive" className={className}>
      <AlertCircle className="h-4 w-4" />
      <AlertTitle className="flex items-center justify-between">
        {displayTitle}
        {onDismiss && (
          <Button variant="ghost" size="sm" className="h-6 w-6 p-0 ml-2" onClick={onDismiss}>
            <X className="h-4 w-4" />
          </Button>
        )}
      </AlertTitle>
      <AlertDescription>
        <div className="space-y-2">
          <p>{errorDetails.message}</p>
          {showValidationDetails &&
            errorDetails.validationErrors &&
            Object.keys(errorDetails.validationErrors).length > 0 && (
              <div className="mt-2 border-t border-destructive/20 pt-2">
                <p className="font-medium mb-1">Validation errors:</p>
                <ul className="list-disc list-inside space-y-1 text-sm">
                  {Object.entries(errorDetails.validationErrors).map(([field, error]) => (
                    <li key={field}>
                      <span className="font-medium">{field}:</span> {error}
                    </li>
                  ))}
                </ul>
              </div>
            )}
        </div>
      </AlertDescription>
    </Alert>
  )
}
