import { createRoute, z } from '@hono/zod-openapi'

/**
 * Schema for updating task metadata via MCP tools
 * Used by sandbox-to-backend communication with task-scoped JWT auth
 */
export const UpdateTaskMetadataRequestSchema = z.object({
  name: z.string().optional().describe('URL-friendly task name (alphanumeric and dashes only)'),
  description: z.string().optional().describe('One-sentence summary of the task'),
})

export const UpdateTaskMetadataResponseSchema = z.object({
  success: z.boolean(),
})

/**
 * OpenAPI route definition for updateTaskMetadata endpoint
 * POST /api/v1/tasks/:taskId/metadata
 *
 * Requires X-Task-Token header with task-scoped JWT
 */
export const updateTaskMetadataRoute = createRoute({
  method: 'post',
  path: '/api/v1/tasks/{taskId}/metadata',
  tags: ['Task MCP'],
  summary: 'Update task metadata',
  description: 'Update task name and description. Requires task-scoped JWT token.',
  security: [
    {
      TaskToken: [],
    },
  ],
  request: {
    params: z.object({
      taskId: z.string().openapi({
        param: {
          name: 'taskId',
          in: 'path',
          required: true,
        },
        example: '123e4567-e89b-12d3-a456-426614174000',
      }),
    }),
    body: {
      content: {
        'application/json': {
          schema: UpdateTaskMetadataRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Task metadata updated successfully',
      content: {
        'application/json': {
          schema: UpdateTaskMetadataResponseSchema,
        },
      },
    },
    401: {
      description: 'Unauthorized - Invalid or missing task token',
    },
    403: {
      description: 'Forbidden - Token not valid for this task',
    },
    404: {
      description: 'Task not found',
    },
  },
})
