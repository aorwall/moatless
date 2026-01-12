import type { Context, Next } from 'hono'
import { HTTPException } from 'hono/http-exception'
import * as jose from 'jose'
import { logger } from '../utils/logger'

/**
 * Task-scoped authentication middleware
 * Validates JWT tokens issued by the backend for sandbox-to-backend communication
 *
 * Token format:
 * - Header: X-Task-Token: <jwt>
 * - Payload: { taskId: string, userId: string, exp: number }
 * - Secret: TASK_TOKEN_SECRET environment variable
 *
 * Usage:
 * ```typescript
 * app.openapi(updateTaskMetadataRoute, validateTaskToken, async (c) => {
 *   const claims = c.get('taskClaims')
 *   // claims.taskId and claims.userId are available
 * })
 * ```
 */
export const validateTaskToken = async (c: Context, next: Next) => {
  const token = c.req.header('X-Task-Token')

  if (!token) {
    throw new HTTPException(401, { message: 'Task token required' })
  }

  const secret = process.env['TASK_TOKEN_SECRET']
  if (!secret) {
    logger.error('TASK_TOKEN_SECRET not configured')
    throw new HTTPException(500, { message: 'Server configuration error' })
  }

  try {
    // Verify JWT signature and expiration
    const secretKey = new TextEncoder().encode(secret)
    const { payload } = await jose.jwtVerify(token, secretKey)

    // Validate payload structure
    const taskId = payload['taskId']
    const userId = payload['userId']

    if (typeof taskId !== 'string' || typeof userId !== 'string') {
      throw new HTTPException(401, { message: 'Invalid token payload' })
    }

    // Verify taskId matches route parameter
    const routeTaskId = c.req.param('taskId')
    if (routeTaskId && taskId !== routeTaskId) {
      logger.warn(
        {
          tokenTaskId: taskId,
          routeTaskId,
        },
        'Token taskId mismatch',
      )
      throw new HTTPException(403, {
        message: 'Token not valid for this task',
      })
    }

    // Inject claims into context
    c.set('taskClaims', {
      taskId,
      userId,
    })

    logger.debug(
      {
        taskId,
        userId,
      },
      'Task token validated',
    )

    await next()
  } catch (error) {
    if (error instanceof HTTPException) {
      throw error
    }

    // JWT verification errors (expired, invalid signature, etc.)
    if (error instanceof jose.errors.JWTExpired) {
      logger.warn({ err: error }, 'Task token expired')
      throw new HTTPException(401, { message: 'Token expired' })
    }

    if (error instanceof jose.errors.JWTInvalid) {
      logger.warn({ err: error }, 'Invalid task token')
      throw new HTTPException(401, { message: 'Invalid token' })
    }

    logger.error({ err: error as Error }, 'Task token validation error')
    throw new HTTPException(401, { message: 'Token validation failed' })
  }
}

/**
 * Helper function to get task claims from context
 */
export function getTaskClaims(c: Context):
  | {
      taskId: string
      userId: string
    }
  | undefined {
  return c.get('taskClaims')
}
