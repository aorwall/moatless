import { createRoute } from '@hono/zod-openapi'
import {
  CommandListResponseSchema,
  CommandSchema,
  CreateCommandSchema,
  UpdateCommandSchema,
} from '@moatless/types'
import { z } from 'zod'

const commandNameParams = z.object({ commandName: z.string().min(1) })

// POST /api/v1/commands - Create command
export const createCommandRoute = createRoute({
  method: 'post',
  path: '/api/v1/commands',
  tags: ['Commands'],
  description: 'Create a new custom command for task automation and workflow execution',
  request: {
    body: {
      content: {
        'application/json': {
          schema: CreateCommandSchema,
        },
      },
      description: 'Command configuration including name, shell script, and execution parameters',
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: CommandSchema,
        },
      },
    },
  },
})

// GET /api/v1/commands - List commands
export const listCommandsRoute = createRoute({
  method: 'get',
  path: '/api/v1/commands',
  tags: ['Commands'],
  description: 'List all configured custom commands available for task execution',
  request: {
    query: z.object({}),
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: CommandListResponseSchema,
        },
      },
    },
  },
})

// GET /api/v1/commands/{commandName} - Get command by name
export const getCommandRoute = createRoute({
  method: 'get',
  path: '/api/v1/commands/{commandName}',
  tags: ['Commands'],
  description: 'Retrieve a specific custom command by its unique name',
  request: {
    params: commandNameParams,
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: CommandSchema,
        },
      },
    },
  },
})

// PUT /api/v1/commands/{commandName} - Update command
export const updateCommandRoute = createRoute({
  method: 'put',
  path: '/api/v1/commands/{commandName}',
  tags: ['Commands'],
  description: 'Update an existing custom command configuration',
  request: {
    params: commandNameParams,
    body: {
      content: {
        'application/json': {
          schema: UpdateCommandSchema,
        },
      },
      description: 'Updated command configuration including script and parameters',
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: CommandSchema,
        },
      },
    },
  },
})

// DELETE /api/v1/commands/{commandName} - Delete command
export const deleteCommandRoute = createRoute({
  method: 'delete',
  path: '/api/v1/commands/{commandName}',
  tags: ['Commands'],
  description: 'Delete a custom command by its name',
  request: {
    params: commandNameParams,
  },
  responses: {
    204: { description: 'Command deleted successfully' },
    400: { description: 'Bad Request - Invalid input' },
    404: { description: 'Not Found - Resource does not exist' },
    500: { description: 'Internal Server Error' },
  },
})
