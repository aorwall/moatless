import { MoatlessError } from '@moatless/shared'
import type { ErrorHandler } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'

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
      error.statusCode as any,
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
  if (error.name === 'ZodError') {
    return c.json(
      {
        error: {
          code: 'VALIDATION_ERROR',
          message: 'Request validation failed',
          details: (error as any).issues,
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

  // Log unexpected errors
  console.error('Unexpected error:', error)

  return c.json(
    {
      error: {
        code: 'INTERNAL_SERVER_ERROR',
        message: 'Internal server error',
      },
      requestId,
    },
    500,
  )
}
