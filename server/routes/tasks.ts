import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { TaskNotFoundError, WorkspaceError } from '../../shared/constants/errors.js'
import {
  CreateTaskRequestSchema,
  DeleteTaskResponseSchema,
  type EventListResponse,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  SimpleTaskResponseSchema,
  TaskListQuerySchema,
  type TaskListResponse,
  UpdateTaskRequestSchema,
  UpdateTaskResponseSchema,
} from '../../shared/types/api.js'
import type { EventType } from '../../shared/types/event.js'
import { sseManager } from '../utils/sse-manager.js'

// Schema for URL params
const TaskIdParamsSchema = z.object({
  taskId: z.string(),
})

export async function registerTaskRoutes(app: FastifyInstance) {
  const { task: taskService, event: eventService } = app.services

  // Create a new task
  app.post(
    '/',
    {
      schema: {
        body: CreateTaskRequestSchema,
        response: {
          200: SimpleTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const taskId = await taskService.createTask(request.body as any)
      return { taskId, status: 'active' }
    },
  )

  // Get task by ID
  app.get(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const task = await taskService.getTask(taskId)

      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      return task
    },
  )

  // List all tasks with pagination
  app.get(
    '/',
    {
      schema: {
        querystring: TaskListQuerySchema,
      },
    },
    async (request, _reply) => {
      const { limit, offset, status } = request.query as any
      let tasks = await taskService.listTasks(limit, offset)

      // Filter by status if provided
      if (status) {
        tasks = tasks.filter((task) => task.status === status)
      }

      const response: TaskListResponse = {
        tasks: tasks,
        items: tasks,
        pagination: {
          limit,
          offset,
          total: tasks.length, // Note: This is simplified, real implementation would need total count
        },
      }
      return response
    },
  )

  // Send message to task
  app.post(
    '/:taskId/messages',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: SendMessageRequestSchema,
        response: {
          200: SendMessageResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { message } = request.body as any

      await taskService.sendMessage(taskId, message)
      return { status: 'sent', timestamp: new Date().toISOString() }
    },
  )

  // Get task events without pagination
  app.get(
    '/:taskId/events',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: z.object({
          type: z.string().optional(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { type } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      const events = await eventService.getEvents(
        taskId,
        undefined, // no limit
        0, // offset 0
        type as EventType | undefined,
        task,
      )

      const response: EventListResponse = {
        events,
        items: events,
      }
      return response
    },
  )

  // Update task
  app.patch(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: UpdateTaskRequestSchema,
        response: {
          200: UpdateTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const updates = request.body as any

      await taskService.updateTask(taskId, updates)
      return { status: 'updated' }
    },
  )

  // Delete task
  app.delete(
    '/:taskId',
    {
      schema: {
        params: TaskIdParamsSchema,
        response: {
          200: DeleteTaskResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      await taskService.deleteTask(taskId)
      return { status: 'deleted' }
    },
  )

  // Get workspace status
  app.get(
    '/:taskId/workspace-status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Get workspace status from workspace service
      const workspaceStatus = await app.services.workspace.getWorkspaceStatus(taskId)
      return workspaceStatus
    },
  )

  // List files in workspace
  app.get(
    '/:taskId/files',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: z.object({
          path: z.string().optional(),
          recursive: z.coerce.boolean().optional(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { path = '/', recursive = false } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status === 'not_setup') {
        throw new WorkspaceError('Workspace not created', taskId)
      }

      // List files
      const files = await app.services.workspace.listFiles(taskId, path, recursive)
      return { files, totalCount: files.length }
    },
  )

  // Read file content
  app.get(
    '/:taskId/files/*',
    {
      schema: {
        params: z.object({
          taskId: z.string(),
          '*': z.string(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const filePath = (request.params as any)['*']

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status === 'not_setup') {
        throw new WorkspaceError('Workspace not created', taskId)
      }

      // Read file
      const content = await app.services.workspace.readFile(taskId, filePath)
      return { content, encoding: 'utf-8' }
    },
  )

  // Write file content
  app.put(
    '/:taskId/files/*',
    {
      schema: {
        params: z.object({
          taskId: z.string(),
          '*': z.string(),
        }),
        body: z.object({
          content: z.string(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const filePath = (request.params as any)['*']
      const { content } = request.body as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status === 'not_setup') {
        throw new WorkspaceError('Workspace not created', taskId)
      }

      // Write file
      await app.services.workspace.writeFile(taskId, filePath, content)
      return { success: true }
    },
  )

  // Search files by filename/path
  app.post(
    '/:taskId/search-files',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: z.object({
          query: z.string(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { query } = request.body as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status === 'not_setup') {
        throw new WorkspaceError('Workspace not created', taskId)
      }

      // Search files
      const result = await app.services.workspace.searchFiles(taskId, {
        query,
      })
      return result
    },
  )

  // Get git diff for workspace
  app.get(
    '/:taskId/git-diff',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status === 'not_setup') {
        throw new WorkspaceError('Workspace not created', taskId)
      }

      // Get git diff
      const diff = await app.services.workspace.getGitDiff(taskId)
      return {
        diff,
        files: [], // TODO: Parse diff to extract file list
        hasChanges: diff.length > 0,
      }
    },
  )

  // Execute command in workspace
  app.post(
    '/:taskId/execute',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: z.object({
          command: z.string(),
          timeout: z.number().optional(),
          env: z.record(z.string()).optional(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { command, timeout } = request.body as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state - only allow in IDLE state
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status !== 'idle') {
        throw new WorkspaceError(`Cannot execute command: workspace is ${status.status}`, taskId)
      }

      // Execute command
      const result = await app.services.workspace.runCommand(taskId, command, timeout)
      return result
    },
  )

  // Start development server
  app.post(
    '/:taskId/server/start',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: z.object({
          command: z.string().optional(),
          port: z.number().optional(),
          env: z.record(z.string()).optional(),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { port } = request.body as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Check workspace state - allow in IDLE or RUNNING
      const status = await app.services.workspace.getWorkspaceStatus(taskId)
      if (status.status !== 'idle' && status.status !== 'running') {
        throw new WorkspaceError(`Cannot start server: workspace is ${status.status}`, taskId)
      }

      // Start server
      const result = await app.services.workspace.startServer(taskId)
      return {
        running: true,
        port: result.port || port || 3000,
        pid: result.pid,
        startedAt: new Date().toISOString(),
      }
    },
  )

  // Stop development server
  app.post(
    '/:taskId/server/stop',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Stop server
      const stopped = await app.services.workspace.stopServer(taskId)
      return {
        stopped,
        runtime: 0, // TODO: Calculate actual runtime
      }
    },
  )

  // Get server status
  app.get(
    '/:taskId/server/status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      return await app.services.workspace.getServerStatus(taskId)
    },
  )

  // Get server logs
  app.get(
    '/:taskId/server/logs',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: z.object({
          lines: z.coerce.number().optional().default(100),
          follow: z.coerce.boolean().optional().default(false),
        }),
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const { lines } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Get server logs
      const logs = await app.services.workspace.getServerLogs(taskId, lines)
      return {
        logs,
        lines: logs.split('\n').length,
        truncated: false, // TODO: Implement truncation logic
      }
    },
  )

  // SSE endpoint for real-time events
  app.get(
    '/:taskId/stream',
    {
      schema: {
        params: TaskIdParamsSchema,
        querystring: z.object({
          lastEventId: z.string().optional(),
        }),
      },
    },
    async (request, reply) => {
      const { taskId } = request.params as any
      const { lastEventId } = request.query as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Set SSE headers
      reply.raw.writeHead(200, {
        'Content-Type': 'application/x-ndjson',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
      })

      // Register connection with manager
      const connectionId = sseManager.connect(taskId, request, reply, lastEventId)

      let offset = 0

      // If lastEventId is provided, find the offset
      if (lastEventId) {
        const allEvents = await eventService.getEvents(taskId, undefined, 0, undefined, task)
        for (let i = 0; i < allEvents.length; i++) {
          if (allEvents[i].eventId === lastEventId) {
            offset = i + 1
            break
          }
        }
      }

      // Send existing events from offset
      const existingEvents = await eventService.getEvents(taskId, 1000, offset, undefined, task)
      for (const event of existingEvents) {
        reply.raw.write(JSON.stringify(event) + '\n')
      }

      // Subscribe to new events
      const unsubscribe = eventService.subscribe(taskId, (event) => {
        try {
          reply.raw.write(JSON.stringify(event) + '\n')
        } catch (error) {
          request.log.error('Failed to send event over SSE', error)
        }
      })

      // Handle client disconnect
      request.raw.on('close', () => {
        unsubscribe()
        sseManager.disconnect(taskId, connectionId)
        reply.raw.end()
      })

      // Keep connection alive with periodic pings
      const pingInterval = setInterval(() => {
        try {
          reply.raw.write(':\n') // SSE comment for keep-alive
        } catch (error) {
          clearInterval(pingInterval)
        }
      }, 30000)

      request.raw.on('close', () => {
        clearInterval(pingInterval)
      })
    },
  )

  // Get SSE stream status
  app.get(
    '/:taskId/stream/status',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      return {
        activeConnections: sseManager.getConnectionCount(taskId),
        taskActive: task.status === 'active',
      }
    },
  )
}
