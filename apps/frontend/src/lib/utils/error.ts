import { ApiError } from '@/lib/api/client'

/**
 * Extracts error message from API error data object
 */
function getApiErrorMessage(data: Record<string, unknown>): string | null {
  // Check for common error message fields
  if (typeof data.detail === 'string') {
    return data.detail
  }
  if (typeof data.message === 'string') {
    return data.message
  }
  if (typeof data.error === 'string') {
    return data.error
  }
  return null
}

/**
 * Handles ApiError instances
 */
function handleApiError(error: ApiError): string {
  // Handle API errors with structured data
  if (error.data && typeof error.data === 'object') {
    const data = error.data as Record<string, unknown>
    const message = getApiErrorMessage(data)
    if (message) {
      return message
    }
  }

  // Fallback to status text for API errors
  return error.statusText || `Request failed with status ${error.status}`
}

/**
 * Extracts a user-friendly error message from an error object
 */
export function getErrorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return handleApiError(error)
  }

  // Handle standard Error objects
  if (error instanceof Error) {
    return error.message
  }

  // Handle string errors
  if (typeof error === 'string') {
    return error
  }

  // Fallback for unknown error types
  return 'An unexpected error occurred'
}
