import {
  getErrorMessage,
  getValidationErrors,
  isApiErrorResponse,
  isValidationError,
} from '@moatless/shared/types/api-errors'
import { useCallback } from 'react'
import { ApiError } from '@/lib/api/client'

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

export function useApiError(): UseApiErrorResult {
  const formatErrorForDisplay = useCallback((error: unknown) => {
    // Handle ApiError from our client
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

    // Handle raw API error response
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

    // Fallback for other errors
    return {
      title: 'Error',
      message: getErrorMessage(error),
    }
  }, [])

  const checkIsValidationError = useCallback((error: unknown): boolean => {
    if (error instanceof ApiError && error.apiError) {
      return isValidationError(error.apiError.error)
    }

    if (isApiErrorResponse(error)) {
      return isValidationError(error.error)
    }

    return false
  }, [])

  const extractValidationErrors = useCallback((error: unknown): Record<string, string> => {
    if (error instanceof ApiError && error.apiError) {
      return getValidationErrors(error.apiError)
    }

    return getValidationErrors(error)
  }, [])

  return {
    getErrorMessage,
    getValidationErrors: extractValidationErrors,
    isValidationError: checkIsValidationError,
    formatErrorForDisplay,
  }
}
