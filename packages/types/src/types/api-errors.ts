export interface ApiErrorResponse {
  error: ApiResponseError
  requestId?: string
}

export interface ApiResponseError {
  code: string
  message: string
  details?: Record<string, unknown> | unknown[]
}

export interface ApiValidationError extends Omit<ApiResponseError, 'details'> {
  code: 'VALIDATION_ERROR'
  details:
    | {
        path: string | string[]
        message: string
      }[] // Direct array format (actual backend response)
    | {
        errors:
          | Record<string, string>
          | {
              path: string | string[]
              message: string
            }[]
      } // Legacy/alternative format
}

// Type guard functions
export function isValidationError(error: ApiResponseError): error is ApiValidationError {
  return error.code === 'VALIDATION_ERROR'
}

export function isApiErrorResponse(error: unknown): error is ApiErrorResponse {
  return (
    typeof error === 'object' &&
    error !== null &&
    'error' in error &&
    typeof (error as ApiErrorResponse).error === 'object' &&
    'code' in (error as ApiErrorResponse).error &&
    'message' in (error as ApiErrorResponse).error
  )
}

// Helper to extract user-friendly error messages
export function getErrorMessage(error: unknown): string {
  if (isApiErrorResponse(error)) {
    return error.error.message
  }

  if (error instanceof Error) {
    return error.message
  }

  return 'An unexpected error occurred'
}

// Helper to format error path
function formatErrorPath(path: string | string[]): string {
  return Array.isArray(path) ? path.join('.') : String(path)
}

// Helper to check if value is a validation error item
function isValidationErrorItem(
  value: unknown,
): value is { path: string | string[]; message: string } {
  return typeof value === 'object' && value !== null && 'path' in value && 'message' in value
}

// Helper to process array of validation errors
function processErrorArray(errorArray: unknown[]): Record<string, string> {
  const errors: Record<string, string> = {}
  for (const err of errorArray) {
    if (isValidationErrorItem(err)) {
      errors[formatErrorPath(err.path)] = String(err.message)
    }
  }
  return errors
}

// Helper to extract validation errors for form fields
export function getValidationErrors(error: unknown): Record<string, string> {
  if (!(isApiErrorResponse(error) && isValidationError(error.error))) {
    return {}
  }

  // Handle direct array format (actual backend response)
  if (Array.isArray(error.error.details)) {
    return processErrorArray(error.error.details)
  }

  // Handle nested structure (legacy/alternative format)
  const details = error.error.details?.errors

  if (Array.isArray(details)) {
    return processErrorArray(details)
  }

  if (details && typeof details === 'object') {
    return details as Record<string, string>
  }

  return {}
}
