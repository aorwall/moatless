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
  StopAgentResponseSchema,
  TaskListQuerySchema,
  type TaskListResponse,
} from '../../shared/types/api.js'
import type { EventType } from '../../shared/types/event.js'
import { sseManager } from '../utils/sse-manager.js'

// Schema for URL params
const TaskIdParamsSchema = z.object({
  taskId: z.string(),
})

// Schema for query params
const EventTypeQuerySchema = z.object({
  type: z.string().optional(),
})

const FileQuerySchema = z.object({
  path: z.string().optional().default('/'),
  recursive: z.boolean().optional().default(false),
})

const FileContentSchema = z.object({
  content: z.string(),
})

const TaskFileParamsSchema = z.object({
  taskId: z.string(),
  '*': z.string(),
})

const FileSearchSchema = z.object({
  query: z.string(),
  limit: z.number().optional(),
})

const CommandExecutionSchema = z.object({
  command: z.string(),
  timeout: z.number().optional(),
})

const ServerLogsQuerySchema = z.object({
  lines: z.number().optional(),
})

const MergeTaskSchema = z.object({
  targetBranch: z.string().optional().default('main'),
  commitMessage: z.string().optional(),
})

const SSEQuerySchema = z.object({
  lastEventId: z.string().optional(),
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
      const { model, filePaths, ...taskData } = request.body as z.infer<
        typeof CreateTaskRequestSchema
      >
      const taskId = await taskService.createTask({ ...taskData, model, filePaths })

      // Get the task to return the current status
      const task = await taskService.getTask(taskId)
      return { taskId, status: task?.status || 'open' }
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
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
      const { limit, offset, status } = request.query as z.infer<typeof TaskListQuerySchema>
      const tasks = await taskService.listTasks(limit, offset, status)

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { message, resume, model, fileIds, filePaths, commandName } = request.body as z.infer<
        typeof SendMessageRequestSchema
      >

      await taskService.sendMessage(taskId, message, resume, model, fileIds, filePaths, commandName)
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { type } = request.query as z.infer<typeof EventTypeQuerySchema>

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

      // Task validation now handled in workspace service
      const workspaceStatus = await app.services.workspace.getWorkspaceStatus(taskId)
      return workspaceStatus
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

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
    async (request, reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { path = '/', recursive = false } = request.query as z.infer<typeof FileQuerySchema>

      try {
        // Task validation and capability check now handled in workspace service
        const files = await app.services.workspace.listFiles(taskId, path, recursive)
        return { files, totalCount: files.length }
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          return reply.status(404).send({ error: 'Task not found' })
        }
        if (error instanceof Error && error.message.includes('Path not found')) {
          return reply.status(404).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Access denied')) {
          return reply.status(403).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot list files')) {
          return reply.status(403).send({ error: error.message })
        }
        throw error
      }
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
    async (request, reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { '*': filePath } = request.params as z.infer<typeof TaskFileParamsSchema>

      if (!filePath) {
        return reply.status(400).send({ error: 'File path is required' })
      }

      try {
        // Task validation and capability check now handled in workspace service
        const content = await app.services.workspace.readFile(taskId, filePath)
        return { content, encoding: 'utf-8' }
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          return reply.status(404).send({ error: 'Task not found' })
        }
        if (error instanceof Error && error.message.includes('File not found')) {
          return reply.status(404).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Access denied')) {
          return reply.status(403).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('File too large')) {
          return reply.status(413).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Path is not a file')) {
          return reply.status(400).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot read file')) {
          return reply.status(403).send({ error: error.message })
        }
        throw error
      }
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { '*': filePath } = request.params as z.infer<typeof TaskFileParamsSchema>
      const { content } = request.body as z.infer<typeof FileContentSchema>

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
          limit: z.number().optional(),
        }),
      },
    },
    async (request, reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { query, limit } = request.body as z.infer<typeof FileSearchSchema>

      try {
        // Task validation and capability check now handled in workspace service
        const result = await app.services.workspace.searchFiles(taskId, {
          query,
          limit,
        })
        return result
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          return reply.status(404).send({ error: 'Task not found' })
        }
        if (error instanceof Error && error.message.includes('Cannot search files')) {
          return reply.status(403).send({ error: error.message })
        }
        throw error
      }
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
    async (request, reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

      try {
        // Task validation and capability check now handled in workspace service
        const diff = await app.services.workspace.getGitDiff(taskId)
        return {
          diff,
          files: [], // TODO: Parse diff to extract file list
          hasChanges: diff.length > 0,
        }
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          return reply.status(404).send({ error: 'Task not found' })
        }
        if (error instanceof WorkspaceError) {
          return reply.status(error.statusCode).send({ error: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot access git')) {
          return reply.status(403).send({ error: error.message })
        }
        throw error
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { command, timeout } = request.body as z.infer<typeof CommandExecutionSchema>

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

      // Task validation and capability check now handled in workspace service
      const stopped = await app.services.workspace.stopServer(taskId)
      return {
        stopped,
        runtime: 0, // TODO: Calculate actual runtime
      }
    },
  )

  // Restart development server
  app.post(
    '/:taskId/server/restart',
    {
      schema: {
        params: TaskIdParamsSchema,
      },
    },
    async (request, _reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

      // Task validation and capability check now handled in workspace service
      const result = await app.services.workspace.restartServer(taskId)
      return result
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { lines } = request.query as z.infer<typeof ServerLogsQuerySchema>

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
          400: z.object({
            statusCode: z.literal(400),
            error: z.string(),
            message: z.string(),
          }),
        },
      },
    },
    async (request, reply) => {
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const body = request.body || {}
      const { targetBranch = 'main', commitMessage } = body as z.infer<typeof MergeTaskSchema>

      // Import git utils
      const { getCurrentBranch, hasUncommittedChanges, mergeGitBranch, checkForConflicts } =
        await import('../utils/git-utils.js')

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

      // Extract main repo path from worktree for additional checks
      const gitPath = `${workspacePath}/.git`
      let mainRepoPath = workspacePath

      try {
        const { exec } = await import('node:child_process')
        const { promisify } = await import('node:util')
        const execAsync = promisify(exec)

        const { stdout: gitFileContent } = await execAsync(`cat "${gitPath}"`, {
          cwd: workspacePath,
          timeout: 5000,
        })

        // If .git is a file (worktree), extract main repo path
        if (gitFileContent.startsWith('gitdir:')) {
          const gitDir = gitFileContent.split(':')[1].trim()
          // Extract main repo path from worktree git dir
          const pathParts = gitDir.split('/')
          const worktreesIndex = pathParts.lastIndexOf('worktrees')
          if (worktreesIndex > 0) {
            mainRepoPath = pathParts.slice(0, worktreesIndex - 1).join('/')
          }
        }
      } catch {
        // .git is a directory, we're in the main repo
      }

      // Check for uncommitted changes in the main repository target branch
      const mainHasChanges = await hasUncommittedChanges(mainRepoPath)
      if (mainHasChanges) {
        reply.code(400)
        return {
          statusCode: 400,
          error: 'Bad Request',
          message: `Cannot merge: target branch '${targetBranch}' has uncommitted changes in the main repository`,
        }
      }

      // Check for conflicts before attempting merge
      const hasConflicts = await checkForConflicts(workspacePath, targetBranch)
      if (hasConflicts) {
        reply.code(400)
        return {
          statusCode: 400,
          error: 'Bad Request',
          message: `Cannot merge: conflicts detected between '${currentBranch}' and '${targetBranch}'`,
        }
      }

      // Perform the merge
      const mergeCommit = await mergeGitBranch(
        workspacePath,
        currentBranch,
        targetBranch,
        commitMessage,
      )

      // Check if worktree has uncommitted changes after merge
      const worktreeHasChanges = await hasUncommittedChanges(workspacePath)

      // If no uncommitted changes on the worktree branch after merge, update task status to "merged"
      if (!worktreeHasChanges) {
        await taskService.updateTask(taskId, { status: 'merged' })
      }

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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>
      const { lastEventId } = request.query as z.infer<typeof SSEQuerySchema>

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Set SSE headers with CORS
      const origin = request.headers.origin
      const corsHeaders: Record<string, string> = {}

      // Add CORS headers based on the same logic as the CORS plugin
      if (origin) {
        // In development, allow all localhost origins
        if (process.env.NODE_ENV !== 'production') {
          try {
            const url = new URL(origin)
            if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
              corsHeaders['Access-Control-Allow-Origin'] = origin
              corsHeaders['Access-Control-Allow-Credentials'] = 'true'
            }
          } catch {
            // Invalid origin URL, skip CORS headers
          }
        } else {
          // In production, use configured origin
          const allowedOrigin = process.env.CORS_ORIGIN
          if (allowedOrigin === 'true' || allowedOrigin === origin) {
            corsHeaders['Access-Control-Allow-Origin'] = origin
            corsHeaders['Access-Control-Allow-Credentials'] = 'true'
          }
        }
      } else if (process.env.CORS_ORIGIN === 'true' || process.env.NODE_ENV !== 'production') {
        // No origin header but CORS allows all origins
        corsHeaders['Access-Control-Allow-Origin'] = '*'
      }

      reply.raw.writeHead(200, {
        'Content-Type': 'application/x-ndjson',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
        'X-Accel-Buffering': 'no', // Disable Nginx buffering
        ...corsHeaders,
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
        reply.raw.write(`${JSON.stringify(event)}\n`)
      }

      // Flush the initial data
      reply.raw.flushHeaders()

      // Subscribe to new events
      const unsubscribe = eventService.subscribe(taskId, (event) => {
        try {
          reply.raw.write(`${JSON.stringify(event)}\n`)
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
            timestamp: new Date().toISOString(),
          }
          reply.raw.write(`${JSON.stringify(pingEvent)}\n`)
        } catch (_error) {
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
      const { taskId } = request.params as z.infer<typeof TaskIdParamsSchema>

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
