import { MoatlessError } from '@moatless/types'
import type { Context } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { logger } from './logger'

/**
 * Centralized error handler for service errors
 * Rethrows MoatlessError subclasses so the middleware error handler
 * can format them with proper error codes and status codes.
 * Wraps unknown errors in HTTPException(500).
 */
export function handleServiceError(error: unknown, context: string): never {
  logger.error({ err: error as Error }, `${context} failed`)

  // Let MoatlessError subclasses pass through to the middleware error handler
  // which formats them with proper error codes (TASK_NOT_FOUND, VALIDATION_ERROR, etc.)
  if (error instanceof MoatlessError) {
    throw error
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
