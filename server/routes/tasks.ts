import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { TaskNotFoundError } from '../../shared/constants/errors.js'
import {
  AgentStatusResponseSchema,
  CreateTaskRequestSchema,
  DeleteTaskResponseSchema,
  type EventListResponse,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  SimpleTaskResponseSchema,
  StopAgentResponseSchema,
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

// Response schema for merge endpoint
const MergeResponseSchema = z.object({
  success: z.boolean(),
  mergeCommit: z.string(),
  sourceBranch: z.string(),
  targetBranch: z.string(),
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
      return { taskId, status: 'open' }
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

      // Task validation now handled in workspace service
      const workspaceStatus = await app.services.workspace.getWorkspaceStatus(taskId)
      return workspaceStatus
    },
  )

  // Get agent status for task
  app.get(
    '/:taskId/agent-status',
    {
      schema: {
        params: TaskIdParamsSchema,
        response: {
          200: AgentStatusResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Get agent status from AgentService
      return await app.services.agent.getAgentStatus(taskId)
    },
  )

  // Stop agent
  app.post(
    '/:taskId/stop',
    {
      schema: {
        params: TaskIdParamsSchema,
        response: {
          200: StopAgentResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Stop agent through AgentService
      const stopped = await app.services.agent.stopAgent(taskId)

      return {
        status: stopped ? 'stopped' : 'not_running',
        taskId,
      }
    },
  )

  // Close task (teardown workspace, stop server, set status to closed)
  app.post(
    '/:taskId/close',
    {
      schema: {
        params: TaskIdParamsSchema,
        response: {
          200: z.object({
            status: z.literal('closed'),
            taskId: z.string(),
          }),
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      await taskService.closeTask(taskId)

      return {
        status: 'closed',
        taskId,
      }
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

      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
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
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any

      // Task validation and capability check now handled in workspace service
      const result = await app.services.workspace.startServer(taskId)
      return result
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

      // Task validation and capability check now handled in workspace service
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
      // Task validation and capability check now handled in workspace service
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

      // Task validation and capability check now handled in workspace service
      const logs = await app.services.workspace.getServerLogs(taskId, lines)
      return {
        logs,
        lines: logs.split('\n').length,
        truncated: false, // TODO: Implement truncation logic
      }
    },
  )

  // Merge task changes
  app.post(
    '/:taskId/merge',
    {
      schema: {
        params: TaskIdParamsSchema,
        body: z
          .object({
            targetBranch: z.string().optional(),
            commitMessage: z.string().optional(),
          })
          .nullable()
          .optional(),
        response: {
          200: MergeResponseSchema,
        },
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as any
      const body = request.body || {}
      const { targetBranch = 'main', commitMessage } = body as any

      // Import git utils
      const { getCurrentBranch, hasUncommittedChanges, mergeGitBranch } = await import(
        '../utils/git-utils.js'
      )

      // Get task to verify it exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Get workspace through workspace service
      const workspace = await app.services.workspace.getOrCreateWorkspace(taskId)
      if (!workspace) {
        throw new Error(`No workspace found for task ${taskId}`)
      }

      const workspacePath = workspace.getWorkspacePath()

      // Get current branch
      const currentBranch = await getCurrentBranch(workspacePath)

      // Check for uncommitted changes
      const hasChanges = await hasUncommittedChanges(workspacePath)
      if (hasChanges) {
        throw new Error('Cannot merge: workspace has uncommitted changes')
      }

      // Perform the merge
      const mergeCommit = await mergeGitBranch(
        workspacePath,
        currentBranch,
        targetBranch,
        commitMessage,
      )

      return {
        success: true,
        mergeCommit,
        sourceBranch: currentBranch,
        targetBranch,
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
      
      // Flush the initial data
      reply.raw.flushHeaders()

      // Subscribe to new events
      const unsubscribe = eventService.subscribe(taskId, (event) => {
        try {
          reply.raw.write(JSON.stringify(event) + '\n')
        } catch (error) {
          request.log.error('Failed to send event over SSE', error)
        }
      })

      // Keep connection alive with periodic pings
      const pingInterval = setInterval(() => {
        try {
          // Send a ping event for NDJSON format
          const pingEvent = {
            type: 'ping',
            timestamp: new Date().toISOString()
          }
          reply.raw.write(JSON.stringify(pingEvent) + '\n')
        } catch (error) {
          clearInterval(pingInterval)
        }
      }, 30000)

      // Handle client disconnect
      request.raw.on('close', () => {
        unsubscribe()
        sseManager.disconnect(taskId, connectionId)
        clearInterval(pingInterval)
        reply.raw.end()
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
        taskActive: task.status === 'open' || task.status === 'running',
      }
    },
  )
}
