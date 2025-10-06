import type { Context } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { z } from 'zod'

/**
 * Standard pagination response format
 */
export interface PaginationInfo {
  limit: number
  offset: number
  total?: number
  hasMore?: boolean
}

export interface PaginatedResponse<T> {
  items: T[]
  pagination: PaginationInfo
}

/**
 * Helper to validate request body with schema
 */
export async function validateRequestBody<T extends z.ZodType>(
  c: Context,
  schema: T,
): Promise<z.infer<T>> {
  try {
    const body = await c.req.json()
    return schema.parse(body)
  } catch (error) {
    if (error instanceof Error) {
      throw new HTTPException(400, {
        message: `Invalid request body: ${error.message}`,
      })
    }
    throw new HTTPException(400, { message: 'Invalid request body' })
  }
}

/**
 * Helper to validate query parameters with schema
 */
export function validateQueryParams<T extends z.ZodType>(c: Context, schema: T): z.infer<T> {
  try {
    const queryParams = c.req.query()
    return schema.parse(queryParams)
  } catch (error) {
    if (error instanceof Error) {
      throw new HTTPException(400, {
        message: `Invalid query parameters: ${error.message}`,
      })
    }
    throw new HTTPException(400, { message: 'Invalid query parameters' })
  }
}

/**
 * Standard OpenAPI parameter definitions
 */
export const CommonParameters = {
  taskId: {
    name: 'taskId',
    in: 'path' as const,
    required: true,
    description: 'Task ID',
    schema: {
      type: 'string' as const,
    },
  },
  repositoryId: {
    name: 'repositoryId',
    in: 'path' as const,
    required: true,
    description: 'Repository ID',
    schema: {
      type: 'string' as const,
    },
  },
  environmentId: {
    name: 'environmentId',
    in: 'path' as const,
    required: true,
    description: 'Environment ID',
    schema: {
      type: 'string' as const,
    },
  },
  limit: {
    name: 'limit',
    in: 'query' as const,
    required: false,
    description: 'Number of items to return (1-100)',
    schema: {
      type: 'number' as const,
      minimum: 1,
      maximum: 100,
      default: 20,
    },
  },
  offset: {
    name: 'offset',
    in: 'query' as const,
    required: false,
    description: 'Number of items to skip',
    schema: {
      type: 'number' as const,
      minimum: 0,
      default: 0,
    },
  },
}

/**
 * Standard OpenAPI response definitions
 */
export const CommonResponses = {
  400: {
    description: 'Invalid request',
    content: {
      'application/json': {
        schema: {
          type: 'object' as const,
          properties: {
            error: {
              type: 'object' as const,
              properties: {
                code: { type: 'string' as const },
                message: { type: 'string' as const },
                details: {},
              },
              required: ['code', 'message'],
            },
          },
          required: ['error'],
        },
      },
    },
  },
  404: {
    description: 'Resource not found',
    content: {
      'application/json': {
        schema: {
          type: 'object' as const,
          properties: {
            error: {
              type: 'object' as const,
              properties: {
                code: { type: 'string' as const },
                message: { type: 'string' as const },
              },
              required: ['code', 'message'],
            },
          },
          required: ['error'],
        },
      },
    },
  },
  500: {
    description: 'Internal server error',
    content: {
      'application/json': {
        schema: {
          type: 'object' as const,
          properties: {
            error: {
              type: 'object' as const,
              properties: {
                code: { type: 'string' as const },
                message: { type: 'string' as const },
                details: {},
              },
              required: ['code', 'message'],
            },
          },
          required: ['error'],
        },
      },
    },
  },
}

/**
 * Helper to extract file path from wildcard route parameters
 */
export function extractFilePathFromWildcard(
  requestPath: string,
  prefix: string,
  taskId: string,
): string {
  const basePattern = `${prefix}/${taskId}/files/`
  const encodedFilePath = requestPath.replace(basePattern, '')
  const filePath = decodeURIComponent(encodedFilePath)

  if (!filePath) {
    throw new HTTPException(400, { message: 'File path is required' })
  }

  return filePath
}
