import { createRoute } from '@hono/zod-openapi'
import { EventSchema } from '@moatless/types'
import { z } from 'zod'
import { withStandardErrors } from '../responses.js'

// Success response schema for worker event emission
const EmitEventResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
})

// POST /api/v1/worker/emit-event - Emit event from worker to main API
export const emitWorkerEventRoute = createRoute({
  method: 'post',
  path: '/api/v1/worker/emit-event',
  tags: ['Worker'],
  description: 'Emit an event from a worker process to the main API for task event streaming',
  request: {
    body: {
      content: {
        'application/json': {
          schema: EventSchema,
        },
      },
      description: 'Event data including task ID, type, and payload',
    },
  },
  responses: withStandardErrors(
    EmitEventResponseSchema,
    'Event emitted successfully and broadcast to connected clients',
  ),
})
