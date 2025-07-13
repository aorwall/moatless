import { TaskNotFoundError, ValidationError, WorkspaceError } from '@moatless/shared'
import type { Context } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { logger } from './logger.js'

/**
 * Centralized error handler for service errors
 * Converts service errors to appropriate HTTP exceptions
 */
export function handleServiceError(error: unknown, context: string): never {
  logger.error(`${context} failed`, error as Error)

  if (error instanceof ValidationError) {
    throw new HTTPException(400, {
      message: error.message,
      cause: error,
    })
  }

  if (error instanceof TaskNotFoundError) {
    throw new HTTPException(404, {
      message: error.message,
      cause: error,
    })
  }

  if (error instanceof WorkspaceError) {
    throw new HTTPException(error.statusCode as any, {
      message: error.message,
      cause: error,
    })
  }

  // Handle specific workspace error messages
  if (error instanceof Error) {
    if (error.message.includes('Path not found')) {
      throw new HTTPException(404, { message: error.message })
    }
    if (error.message.includes('Access denied') || error.message.includes('Cannot read file')) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('File not found')) {
      throw new HTTPException(404, { message: error.message })
    }
    if (error.message.includes('File too large')) {
      throw new HTTPException(413, { message: error.message })
    }
    if (error.message.includes('Path is not a file')) {
      throw new HTTPException(400, { message: error.message })
    }
    if (
      error.message.includes('Cannot list files') ||
      error.message.includes('Cannot search files')
    ) {
      throw new HTTPException(403, { message: error.message })
    }
    if (error.message.includes('Cannot access git')) {
      throw new HTTPException(403, { message: error.message })
    }
  }

  // For all other errors, return 500 with the actual error message
  const errorMessage = error instanceof Error ? error.message : `Failed to ${context.toLowerCase()}`
  throw new HTTPException(500, {
    message: errorMessage,
    cause: error,
  })
}

/**
 * Helper function to validate and extract required path parameter
 */
export function getRequiredParam(c: Context, paramName: string): string {
  const value = c.req.param(paramName)
  if (!value) {
    throw new HTTPException(400, { message: `${paramName} is required` })
  }
  return value
}

/**
 * Helper function to get task ID from path parameters
 */
export function getTaskId(c: Context): string {
  return getRequiredParam(c, 'taskId')
}

/**
 * Error response middleware for consistent error format
 */
export function createErrorHandler() {
  return async (err: Error, c: Context) => {
    logger.error('Request failed', err)

    if (err instanceof HTTPException) {
      return err.getResponse()
    }

    // Fallback for unhandled errors
    return c.json(
      {
        error: {
          code: 'INTERNAL_SERVER_ERROR',
          message: 'An unexpected error occurred',
          details: process.env.NODE_ENV === 'development' ? err.message : undefined,
        },
      },
      500,
    )
  }
}

/**
 * Standardized error response format
 */
export interface ErrorResponse {
  error: {
    code: string
    message: string
    details?: unknown
  }
}
