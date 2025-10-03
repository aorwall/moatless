import { type ServerStatus, ServerStatusSchema, ServerStopResponseSchema } from '@moatless/types'
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
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result: ServerStatus = await workspace.startServer()
      return c.json(result)
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
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const stopped = await workspace.stopServer()
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
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      await workspace.stopServer()
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const result = await workspace.startServer()
      return c.json(result)
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
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const status = await workspace.getServerStatus()
      return c.json(status)
    },
  )
}
