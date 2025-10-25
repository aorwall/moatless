import { MoatlessError } from '@moatless/types'
import type { ErrorHandler } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { ZodError } from 'zod'
import type { AppBindings } from '../app'

export const errorHandler: ErrorHandler<AppBindings> = (error, c) => {
  const requestId = c.get('requestId')

  if (error instanceof MoatlessError) {
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
