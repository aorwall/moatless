import {
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ServerStatusSchema,
  ServerStopResponseSchema,
} from '@moatless/shared'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'

export async function registerPreviewEnvironmentRoutes(app: Hono<AppBindings>, prefix: string) {
  // Start development server
  app.post(
    `${prefix}/:taskId/server/start`,
    describeRoute({
      description: 'Start development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server started successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.startServer(taskId)
      return c.json(result as any)
    },
  )

  // Stop development server
  app.post(
    `${prefix}/:taskId/server/stop`,
    describeRoute({
      description: 'Stop development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server stopped successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStopResponseSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const stopped = await services.workspace.stopServer(taskId)
      return c.json({
        stopped,
        runtime: 0, // TODO: Calculate actual runtime
      })
    },
  )

  // Restart development server
  app.post(
    `${prefix}/:taskId/server/restart`,
    describeRoute({
      description: 'Restart development server for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server restarted successfully',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.restartServer(taskId)
      return c.json(result as any)
    },
  )

  // Get server status
  app.get(
    `${prefix}/:taskId/server/status`,
    describeRoute({
      description: 'Get development server status for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server status',
          content: {
            'application/json': {
              schema: resolver(ServerStatusSchema),
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      // Task validation and capability check now handled in workspace service
      const status = await services.workspace.getServerStatus(taskId)
      return c.json(status as any)
    },
  )

  // Get server logs
  app.get(
    `${prefix}/:taskId/server/logs`,
    describeRoute({
      description: 'Get development server logs for task',
      parameters: [
        {
          name: 'taskId',
          in: 'path',
          required: true,
          description: 'Task ID',
          schema: {
            type: 'string',
          },
        },
        {
          name: 'lines',
          in: 'query',
          required: false,
          description: 'Number of log lines to return',
          schema: {
            type: 'number',
          },
        },
      ],
      responses: {
        200: {
          description: 'Server logs',
          content: {
            'application/json': {
              schema: resolver(ServerLogsResponseExtendedSchema),
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = c.req.param('taskId')
      if (!taskId) {
        throw new HTTPException(400, { message: 'Task ID is required' })
      }

      const queryParams = c.req.query()
      const validatedQuery = ServerLogsQuerySchema.parse(queryParams)
      const { lines } = validatedQuery

      // Task validation and capability check now handled in workspace service
      const logs = await services.workspace.getServerLogs(taskId, lines)
      return c.json({
        logs,
        lines: logs.split('\n').length,
        truncated: false, // TODO: Implement truncation logic
      })
    },
  )
}
