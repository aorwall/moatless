import { MoatlessError } from '@moatless/types'
import type { ErrorHandler } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { ZodError } from 'zod'
import type { AppBindings } from '../app'
import { logger } from '../utils/logger'

export const errorHandler: ErrorHandler<AppBindings> = (error, c) => {
  const requestId = c.get('requestId')

  if (error instanceof MoatlessError) {
    logger.error('MoatlessError in request', error, {
      requestId,
      code: error.code,
      statusCode: error.statusCode,
    })
    return c.json(
      {
        error: {
          code: error.code,
          message: error.message,
          details: error.details,
        },
        requestId,
      },
      error.statusCode as Parameters<typeof c.json>[1],
    )
  }

  // Handle HTTPException from Hono
  if (error instanceof HTTPException) {
    logger.warn('HTTPException in request', {
      requestId,
      status: error.status,
      message: error.message,
    })
    return c.json(
      {
        error: {
          code: 'HTTP_ERROR',
          message: error.message,
        },
        requestId,
      },
      error.status,
    )
  }

  // Handle Zod validation errors
  if (error instanceof ZodError) {
    logger.warn('Validation error in request', {
      requestId,
      issues: error.issues,
    })
    return c.json(
      {
        error: {
          code: 'VALIDATION_ERROR',
          message: 'Request validation failed',
          details: error.issues,
        },
        requestId,
      },
      400,
    )
  }

  // Handle HTTP errors
  if (error.message?.includes('404')) {
    return c.json(
      {
        error: {
          code: 'NOT_FOUND',
          message: 'Resource not found',
        },
        requestId,
      },
      404,
    )
  }

  // For all other errors, return the actual error message
  const errorMessage = error instanceof Error ? error.message : 'Internal server error'

  // Log all unhandled errors with full details
  logger.error(
    'Unhandled error in request',
    error instanceof Error ? error : new Error(String(error)),
    {
      requestId,
      errorMessage,
      stack: error instanceof Error ? error.stack : undefined,
    },
  )

  return c.json(
    {
      error: {
        code: 'INTERNAL_SERVER_ERROR',
        message: errorMessage,
      },
      requestId,
    },
    500,
  )
}
