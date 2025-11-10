import { z } from 'zod'
import { TaskStatusSchema } from './base.schemas.js'

export const McpResponseSchema = z.record(z.string(), z.unknown())

export const PaginationQuerySchema = z.object({
  limit: z.coerce.number().min(1).max(100).default(20),
  offset: z.coerce.number().min(0).default(0),
})

export const TaskListQuerySchema = PaginationQuerySchema.extend({
  status: TaskStatusSchema.optional(),
})

z.object({
  query: z.string(),
  filePattern: z.string().optional(),
  maxResults: z.number().min(1).max(1000).default(100),
})

z.object({
  limit: z.number(),
  offset: z.number(),
  total: z.number().or(z.undefined()),
  hasMore: z.boolean().or(z.undefined()),
})

export const ApiResponseErrorSchema = z.object({
  code: z.string(),
  message: z.string(),
  details: z.record(z.string(), z.unknown()).optional(),
})

z.object({
  error: ApiResponseErrorSchema,
  requestId: z.string().optional(),
})

ApiResponseErrorSchema.extend({
  code: z.literal('VALIDATION_ERROR'),
  details: z.object({
    errors: z.union([
      z.record(z.string(), z.string()),
      z.array(
        z.object({
          path: z.union([z.string(), z.array(z.string())]),
          message: z.string(),
        }),
      ),
    ]),
  }),
})

ApiResponseErrorSchema.extend({
  code: z.enum(['NOT_FOUND', 'ENVIRONMENT_NOT_FOUND', 'REPOSITORY_NOT_FOUND', 'TASK_NOT_FOUND']),
  details: z
    .object({
      id: z.string().optional(),
      resource: z.string().optional(),
    })
    .optional(),
})

ApiResponseErrorSchema.extend({
  code: z.literal('INTERNAL_SERVER_ERROR'),
  details: z
    .object({
      error: z.string().optional(),
    })
    .optional(),
})
