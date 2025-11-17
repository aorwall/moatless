import { TaskNotFoundError, ValidationError, WorkspaceError } from '@moatless/types'
import type { Context } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { logger } from './logger'

/**
 * Centralized error handler for service errors
 * Converts service errors to appropriate HTTP exceptions
 */
export function handleServiceError(error: unknown, context: string): never {
  logger.error({ err: error as Error }, `${context} failed`)

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
    // Cast statusCode to HTTPException's expected type
    const statusCode = error.statusCode as 400 | 404 | 500 | 503
    throw new HTTPException(statusCode, {
      message: error.message,
      cause: error,
    })
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
