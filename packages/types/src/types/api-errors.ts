export interface ApiErrorResponse {
  error: ApiResponseError
  requestId?: string
}

export interface ApiResponseError {
  code: string
  message: string
  details?: Record<string, unknown>
}

export interface ApiValidationError extends ApiResponseError {
  code: 'VALIDATION_ERROR'
  details: {
    errors:
      | Record<string, string>
      | {
          path: string | string[]
          message: string
        }[]
  }
}

export interface NotFoundError extends ApiResponseError {
  code: 'NOT_FOUND' | 'ENVIRONMENT_NOT_FOUND' | 'REPOSITORY_NOT_FOUND' | 'TASK_NOT_FOUND'
  details?: {
    id?: string
    resource?: string
  }
}

export interface InternalServerError extends ApiResponseError {
  code: 'INTERNAL_SERVER_ERROR'
  details?: {
    error?: string
  }
}

export type TypedApiError =
  | ApiValidationError
  | NotFoundError
  | InternalServerError
  | ApiResponseError

// Type guard functions
export function isValidationError(error: ApiResponseError): error is ApiValidationError {
  return error.code === 'VALIDATION_ERROR'
}

export function isNotFoundError(error: ApiResponseError): error is NotFoundError {
  return error.code.endsWith('_NOT_FOUND') || error.code === 'NOT_FOUND'
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

// Helper to extract validation errors for form fields
export function getValidationErrors(error: unknown): Record<string, string> {
  if (!(isApiErrorResponse(error) && isValidationError(error.error))) {
    return {}
  }

  const errors: Record<string, string> = {}
  const details = error.error.details.errors

  if (Array.isArray(details)) {
    // Zod-style errors
    for (const err of details) {
      const path = Array.isArray(err.path) ? err.path.join('.') : err.path
      errors[path] = err.message
    }
  } else {
    // Object-style errors
    for (const [key, value] of Object.entries(details)) {
      errors[key] = value
    }
  }

  return errors
}
