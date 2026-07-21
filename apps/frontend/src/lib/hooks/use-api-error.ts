import { ApiError } from '@/lib/api/client'
import {
  getErrorMessage,
  getValidationErrors,
  isApiErrorResponse,
  isValidationError,
} from '@/lib/types/api-errors'

interface UseApiErrorResult {
  getErrorMessage: (error: unknown) => string
  getValidationErrors: (error: unknown) => Record<string, string>
  isValidationError: (error: unknown) => boolean
  formatErrorForDisplay: (error: unknown) => {
    title: string
    message: string
    validationErrors?: Record<string, string>
  }
}

function formatErrorForDisplay(error: unknown) {
  try {
    if (error instanceof ApiError && error.apiError) {
      const apiError = error.apiError

      if (isValidationError(apiError.error)) {
        return {
          title: 'Validation Error',
          message: apiError.error.message,
          validationErrors: getValidationErrors(apiError),
        }
      }

      return {
        title: `Error: ${apiError.error.code}`,
        message: apiError.error.message,
      }
    }

    if (isApiErrorResponse(error)) {
      if (isValidationError(error.error)) {
        return {
          title: 'Validation Error',
          message: error.error.message,
          validationErrors: getValidationErrors(error),
        }
      }

      return {
        title: `Error: ${error.error.code}`,
        message: error.error.message,
      }
    }

    return {
      title: 'Error',
      message: getErrorMessage(error),
    }
  } catch (_err) {
    return {
      title: 'Error',
      message: error instanceof Error ? error.message : 'An unexpected error occurred',
    }
  }
}

function checkIsValidationError(error: unknown): boolean {
  if (error instanceof ApiError && error.apiError) {
    return isValidationError(error.apiError.error)
  }

  if (isApiErrorResponse(error)) {
    return isValidationError(error.error)
  }

  return false
}

function extractValidationErrors(error: unknown): Record<string, string> {
  if (error instanceof ApiError && error.apiError) {
    return getValidationErrors(error.apiError)
  }

  return getValidationErrors(error)
}

export function useApiError(): UseApiErrorResult {
  return {
    getErrorMessage,
    getValidationErrors: extractValidationErrors,
    isValidationError: checkIsValidationError,
    formatErrorForDisplay,
  }
}
