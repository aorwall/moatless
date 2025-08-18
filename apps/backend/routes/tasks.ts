import {
  CloseTaskResponseSchema,
  CommandExecutionOptionsSchema,
  CreateTaskRequestSchema,
  CreateVibeTaskRequestSchema,
  DeleteTaskResponseSchema,
  DeployResponseSchema,
  type EventListResponse,
  EventListResponseSchema,
  type EventType,
  EventTypeQuerySchema,
  MergeTaskResponseSchema,
  MergeTaskSchema,
  MessagesResponseSchema,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  type ServerStatus,
  ServerStatusSchema,
  ServerStopResponseSchema,
  SimpleTaskResponseSchema,
  StopAgentResponseSchema,
  TaskCommandSchema,
  TaskCommandsResponseSchema,
  TaskListQuerySchema,
  type TaskListResponse,
  TaskListResponseSchema,
  TaskNotFoundError,
  TaskResponseSchema,
  ValidationError,
} from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import { logger } from '../utils/logger.js'

// All schemas are now imported from @moatless/types

// All response schemas are now imported from @moatless/types

export async function registerTaskRoutes(app: Hono<AppBindings>, prefix: string) {
  // Create a new task
  app.post(
    prefix,
    describeRoute({
      description: 'Create a new task',
      requestBody: {
        description: 'Task creation parameters',
        content: {
          'application/json': {
            schema: resolver(CreateTaskRequestSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Task created successfully',
          content: {
            'application/json': {
              schema: resolver(SimpleTaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const body = await c.req.json()
      const validatedBody = CreateTaskRequestSchema.parse(body)
      const { model, filePaths, ...taskData } = validatedBody

      try {
        const task = await taskService.createTask({ ...taskData, model, filePaths })
        return c.json({ taskId: task.taskId, status: task.status, task })
      } catch (error) {
        handleServiceError(error, 'Create task')
      }
    },
  )

  // Create a new vibe task from template
  app.post(
    `${prefix}/vibe`,
    describeRoute({
      description: 'Create a new task from a Vibe template',
      requestBody: {
        description: 'Vibe task creation parameters',
        content: {
          'application/json': {
            schema: resolver(CreateVibeTaskRequestSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Vibe task created successfully',
          content: {
            'application/json': {
              schema: resolver(SimpleTaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request body',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const body = await c.req.json()
      const validatedBody = CreateVibeTaskRequestSchema.parse(body)
      const { filePaths, ...taskData } = validatedBody

      try {
        const taskId = await taskService.createVibeTask({ ...taskData, filePaths })

        // Get the task to return the current status
        const task = await taskService.getTask(taskId)
        return c.json({ taskId, status: task?.status || 'open', task })
      } catch (error) {
        handleServiceError(error, 'Create vibe task')
      }
    },
  )

  // Get task by ID
  app.get(
    `${prefix}/:taskId`,
    describeRoute({
      description: 'Get task details by ID',
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
          description: 'Task details',
          content: {
            'application/json': {
              schema: resolver(TaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services
      const taskId = getTaskId(c)

      try {
        const task = await taskService.getTask(taskId)
        return c.json(task)
      } catch (error) {
        handleServiceError(error, 'Get task')
      }
    },
  )

  // List all tasks with pagination
  app.get(
    prefix,
    describeRoute({
      description:
        'List all tasks with pagination and filtering. By default, closed tasks are excluded unless a specific status filter is provided.',
      parameters: [
        {
          name: 'limit',
          in: 'query',
          required: false,
          description: 'Number of tasks to return (1-100)',
          schema: {
            type: 'number',
            minimum: 1,
            maximum: 100,
            default: 100,
          },
        },
        {
          name: 'offset',
          in: 'query',
          required: false,
          description: 'Number of tasks to skip',
          schema: {
            type: 'number',
            minimum: 0,
            default: 0,
          },
        },
        {
          name: 'status',
          in: 'query',
          required: false,
          description:
            'Filter by task status. If not provided, closed tasks are excluded by default.',
          schema: {
            type: 'string',
            enum: ['open', 'error', 'merged', 'closed'],
          },
        },
      ],
      responses: {
        200: {
          description: 'List of tasks',
          content: {
            'application/json': {
              schema: resolver(TaskListResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid query parameters',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const queryParams = c.req.query()
      const validatedQuery = TaskListQuerySchema.parse(queryParams)
      const { limit, offset, status } = validatedQuery

      const { tasks, total } = await taskService.listTasks(limit, offset, status)

      const response: TaskListResponse = {
        items: tasks,
        pagination: {
          limit,
          offset,
          total,
          hasMore: total !== undefined ? offset + limit < total : undefined,
        },
      }
      return c.json(response)
    },
  )

  // Delete task
  app.delete(
    `${prefix}/:taskId`,
    describeRoute({
      description: 'Delete a task',
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
          description: 'Task deleted successfully',
          content: {
            'application/json': {
              schema: resolver(DeleteTaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services
      const taskId = getTaskId(c)

      try {
        await taskService.deleteTask(taskId)
        return c.json({ status: 'deleted' })
      } catch (error) {
        handleServiceError(error, 'Delete task')
      }
    },
  )

  // Send message to task
  app.post(
    `${prefix}/:taskId/messages`,
    describeRoute({
      description: 'Send a message to a task',
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
      requestBody: {
        description: 'Message parameters',
        content: {
          'application/json': {
            schema: resolver(SendMessageRequestSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Message sent successfully',
          content: {
            'application/json': {
              schema: resolver(SendMessageResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)

      const body = await c.req.json()
      const validatedBody = SendMessageRequestSchema.parse(body)
      const { message, resume, model, fileIds, filePaths, commandName, agentMode } = validatedBody

      await taskService.sendMessage(
        taskId,
        message,
        resume,
        model,
        fileIds,
        filePaths,
        commandName,
        agentMode,
      )
      return c.json({ status: 'sent', timestamp: new Date().toISOString() })
    },
  )

  // Get available commands for task
  app.get(
    `${prefix}/:taskId/commands`,
    describeRoute({
      description: 'Get available commands for a task based on current context',
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
          description: 'List of available commands',
          content: {
            'application/json': {
              schema: resolver(TaskCommandsResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)

      const commands = await taskService.getAvailableCommands(taskId)
      return c.json({ commands })
    },
  )

  // Send command to task
  app.post(
    `${prefix}/:taskId/commands`,
    describeRoute({
      description: 'Send a command to a task',
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
      requestBody: {
        description: 'Command parameters',
        content: {
          'application/json': {
            schema: resolver(TaskCommandSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Command sent successfully',
          content: {
            'application/json': {
              schema: resolver(SendMessageResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task or command not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)

      const body = await c.req.json()
      const validatedBody = TaskCommandSchema.parse(body)
      const { commandName, resume, model, filePaths } = validatedBody

      await taskService.sendCommand(taskId, commandName, resume, model, filePaths)
      return c.json({ status: 'sent', timestamp: new Date().toISOString() })
    },
  )

  // Get task events without pagination
  app.get(
    `${prefix}/:taskId/events`,
    describeRoute({
      description: 'Get all events for a task',
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
          name: 'type',
          in: 'query',
          required: false,
          description: 'Filter by event type',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'List of task events',
          content: {
            'application/json': {
              schema: resolver(EventListResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService, event: eventService } = services

      const taskId = getTaskId(c)

      const queryParams = c.req.query()
      const validatedQuery = EventTypeQuerySchema.parse(queryParams)
      const { type } = validatedQuery

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
      )

      const response: EventListResponse = {
        items: events,
      }
      return c.json(response)
    },
  )

  // Get task messages with merged tool calls
  app.get(
    `${prefix}/:taskId/messages`,
    describeRoute({
      description: 'Get all messages for a task with merged tool calls and results',
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
          description: 'List of task messages',
          content: {
            'application/json': {
              schema: resolver(MessagesResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')

      const taskId = getTaskId(c)

      try {
        const messages = await services.message.getTaskMessages(taskId)

        return c.json({
          messages,
          totalCount: messages.length,
        })
      } catch (error) {
        if (error instanceof Error) {
          if (error.message.includes('Task not found')) {
            throw new TaskNotFoundError(taskId)
          }
          // Re-throw other errors for proper handling
          throw error
        }
        throw error
      }
    },
  )

  // Start task
  app.post(
    `${prefix}/:taskId/start`,
    describeRoute({
      description: 'Start a task by activating the last iteration with "created" status',
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
          description: 'Task started successfully',
          content: {
            'application/json': {
              schema: {
                type: 'object',
                properties: {
                  status: {
                    type: 'string',
                    enum: ['started'],
                  },
                  taskId: {
                    type: 'string',
                  },
                },
                required: ['status', 'taskId'],
              },
            },
          },
        },
        400: {
          description: 'No iterations with "created" status found or invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)

      try {
        await services.task.startTask(taskId)
        return c.json({
          status: 'started',
          taskId,
        })
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: error.message })
        }
        if (error instanceof ValidationError) {
          throw new HTTPException(400, { message: error.message })
        }
        throw error
      }
    },
  )

  // Stop agent
  app.post(
    `${prefix}/:taskId/stop`,
    describeRoute({
      description: 'Stop the agent running for a task',
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
          description: 'Agent stop status',
          content: {
            'application/json': {
              schema: resolver(StopAgentResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)

      // Stop agent through workspace environment
      const workspace = await services.workspace.getWorkspace(taskId)
      let stopped = false

      if (workspace) {
        stopped = await workspace.stopAgent()
      }

      return c.json({
        status: stopped ? 'stopped' : 'not_running',
        taskId,
      })
    },
  )

  // Close task (teardown workspace, stop server, set status to closed)
  app.post(
    `${prefix}/:taskId/close`,
    describeRoute({
      description: 'Close a task (teardown workspace, stop server, set status to closed)',
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
          description: 'Task closed successfully',
          content: {
            'application/json': {
              schema: resolver(CloseTaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)

      await taskService.closeTask(taskId)

      return c.json({
        status: 'closed',
        taskId,
      })
    },
  )

  // Execute commands in task workspace with streaming output
  app.post(
    `${prefix}/:taskId/execute-commands`,
    describeRoute({
      description: 'Execute commands in a task workspace with streaming output',
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
      requestBody: {
        description: 'Command execution options',
        content: {
          'application/json': {
            schema: resolver(CommandExecutionOptionsSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'SSE stream of command execution events',
          content: {
            'text/event-stream': {
              schema: {
                type: 'string',
                description: 'Server-sent events stream',
              },
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { workspace: workspaceService } = services

      const taskId = getTaskId(c)

      const body = await c.req.json()
      const options = CommandExecutionOptionsSchema.parse(body)

      // Import SSE utility
      const { createSSEStream } = await import('../utils/sse-stream.js')

      // Create SSE stream from command execution generator
      const generator = workspaceService.executeCommandsInTaskWorkspace(taskId, options)

      return createSSEStream(generator, {
        headers: {
          'Cache-Control': 'no-cache',
          'X-Task-Id': taskId,
        },
      })
    },
  )

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
              schema: resolver(ServerStatusSchema) as any,
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

      const taskId = getTaskId(c)

      // Task validation and capability check now handled in workspace service
      const result: ServerStatus = await services.workspace.startServer(taskId)
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
              schema: resolver(ServerStopResponseSchema) as any,
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

      const taskId = getTaskId(c)

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
              schema: resolver(ServerStatusSchema) as any,
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

      const taskId = getTaskId(c)

      // Task validation and capability check now handled in workspace service
      const result = await services.workspace.restartServer(taskId)
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
              schema: resolver(ServerStatusSchema) as any,
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

      const taskId = getTaskId(c)

      // Task validation and capability check now handled in workspace service
      const status = await services.workspace.getServerStatus(taskId)
      return c.json(status)
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
              schema: resolver(ServerLogsResponseExtendedSchema) as any,
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

      const taskId = getTaskId(c)

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

  // Merge task changes
  app.post(
    `${prefix}/:taskId/merge`,
    describeRoute({
      description: 'Merge task changes to target branch',
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
      requestBody: {
        description: 'Merge parameters',
        content: {
          'application/json': {
            schema: resolver(MergeTaskSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Merge completed successfully',
          content: {
            'application/json': {
              schema: resolver(MergeTaskResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request or merge conflicts',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)

      let body: unknown = {}
      try {
        body = await c.req.json()
      } catch {
        // Body is optional for this endpoint
      }
      const validatedBody = MergeTaskSchema.parse(body || {})
      const { targetBranch = 'main', commitMessage } = validatedBody

      // Import git utils
      const { getCurrentBranch, hasUncommittedChanges, mergeGitBranch, checkForConflicts } =
        await import('../utils/git-utils.js')

      // Get task to verify it exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Get workspace through workspace service
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
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
        throw new HTTPException(400, {
          message: `Cannot merge: target branch '${targetBranch}' has uncommitted changes in the main repository`,
        })
      }

      // Check for conflicts before attempting merge
      const hasConflicts = await checkForConflicts(workspacePath, targetBranch)
      if (hasConflicts) {
        throw new HTTPException(400, {
          message: `Cannot merge: conflicts detected between '${currentBranch}' and '${targetBranch}'`,
        })
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

      return c.json({
        success: true,
        mergeCommit,
        sourceBranch: currentBranch,
        targetBranch,
      })
    },
  )

  // MCP (Model Context Protocol) endpoint for task-scoped tools
  app.post(
    `${prefix}/:taskId/mcp`,
    describeRoute({
      description: 'Model Context Protocol endpoint for task-scoped tools and operations',
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
      requestBody: {
        description: 'JSON-RPC request',
        content: {
          'application/json': {
            schema: {
              type: 'object',
              properties: {
                jsonrpc: { type: 'string' },
                method: { type: 'string' },
                params: { type: 'object' },
                id: {},
              },
              required: ['jsonrpc', 'method'],
            },
          },
        },
      },
      responses: {
        200: {
          description: 'MCP JSON-RPC response',
          content: {
            'application/json': {
              schema: {
                type: 'object',
                description: 'MCP protocol response',
              },
            },
          },
        },
        400: {
          description: 'Invalid task ID or MCP request',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)

      try {
        // Verify task exists
        await services.task.getTask(taskId)

        // Parse JSON-RPC request body
        const requestBody = await c.req.json()

        // Handle the request through MCP service
        const response = await services.mcp.handleRequest(taskId, requestBody)

        return c.json(response)
      } catch (error) {
        console.error('MCP request error:', {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        })
        return handleServiceError(error, 'MCP request')
      }
    },
  )

  // Deploy task to Netlify
  app.post(
    `${prefix}/:taskId/deploy`,
    describeRoute({
      description: 'Build and deploy task workspace to Netlify',
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
          description: 'Deployment completed (check success field for actual result)',
          content: {
            'application/json': {
              schema: resolver(DeployResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Missing Netlify auth token or build configuration',
        },
        404: {
          description: 'Task not found',
        },
        500: {
          description: 'Internal server error',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)
      let task: any = null

      try {
        // Get task to check for name
        task = await services.task.getTask(taskId)
        if (!task) {
          throw new TaskNotFoundError(taskId)
        }

        // Get Publish settings
        const publishSettings = services.settings.getPublishSettings()
        if (!publishSettings.netlifyAccessToken) {
          throw new HTTPException(400, {
            message:
              'Netlify access token not configured in settings. Please configure it in Settings > Publish.',
          })
        }

        // Deploy using workspace service, passing existing siteId if available
        const result = await services.workspace.deployToNetlify(
          taskId,
          task.name,
          publishSettings.netlifyAccessToken,
          task.siteId, // Pass existing siteId for site reuse
        )

        // If deployment was successful, update the task with site info and latest iteration with site URL
        if (result.success) {
          try {
            const updatedTask = await services.task.getTask(taskId)
            if (updatedTask) {
              // Update task with site information if we have a new siteId
              if (result.siteId && !updatedTask.siteId) {
                updatedTask.siteId = result.siteId
                updatedTask.siteProvider = 'netlify'
              }

              // Update the latest iteration with the site URL
              if (result.siteUrl && updatedTask.iterations.length > 0) {
                const latestIteration = updatedTask.iterations[updatedTask.iterations.length - 1]
                latestIteration.siteUrl = result.siteUrl
              }

              // Save the updated task
              await services.task.updateTask(taskId, updatedTask)

              logger.info('Updated task with deployment info', {
                taskId,
                siteId: result.siteId,
                siteUrl: result.siteUrl,
                iterationNumber:
                  updatedTask.iterations.length > 0
                    ? updatedTask.iterations[updatedTask.iterations.length - 1].iterationNumber
                    : undefined,
              })
            }
          } catch (updateError) {
            // Log error but don't fail the deployment response
            logger.warn('Failed to update task with deployment info', {
              taskId,
              error: updateError instanceof Error ? updateError.message : String(updateError),
            })
          }
        }

        return c.json(result)
      } catch (error) {
        if (error instanceof HTTPException) {
          throw error
        }
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: error.message })
        }

        // Log the error and return a user-friendly message
        const errorMessage = error instanceof Error ? error.message : 'Unknown deployment error'
        logger.error('Deployment failed', error as Error, {
          taskId,
          taskName: task?.name,
          error: errorMessage,
        })
        return c.json({
          success: false,
          error: errorMessage,
          logs: `Deployment failed: ${errorMessage}`,
        })
      }
    },
  )

  // Get task iteration thumbnail
  app.get(
    `${prefix}/:taskId/iterations/:iterationNumber/thumbnail`,
    describeRoute({
      description: 'Get thumbnail image for a specific task iteration',
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
          name: 'iterationNumber',
          in: 'path',
          required: true,
          description: 'Iteration number',
          schema: {
            type: 'integer',
          },
        },
      ],
      responses: {
        200: {
          description: 'Thumbnail image',
          content: {
            'image/png': {
              schema: {
                type: 'string',
                format: 'binary',
              },
            },
          },
        },
        404: {
          description: 'Task, iteration, or thumbnail not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const { task: taskService } = services

      const taskId = getTaskId(c)
      const iterationNumber = Number.parseInt(c.req.param('iterationNumber'), 10)

      if (isNaN(iterationNumber)) {
        throw new HTTPException(404, { message: 'Invalid iteration number' })
      }

      try {
        // Get the task
        const task = await taskService.getTask(taskId)

        // Find the specific iteration
        const iteration = task.iterations.find((it) => it.iterationNumber === iterationNumber)
        if (!iteration) {
          throw new HTTPException(404, { message: `Iteration ${iterationNumber} not found` })
        }

        if (!iteration.thumbnailPath) {
          throw new HTTPException(404, { message: 'No thumbnail available for this iteration' })
        }

        // Read the thumbnail file
        const fs = await import('node:fs/promises')
        try {
          const imageBuffer = await fs.readFile(iteration.thumbnailPath)

          // Set headers for immutable image response with aggressive caching
          c.header('Content-Type', 'image/png')
          c.header('Cache-Control', 'public, max-age=31536000, immutable') // 1 year, immutable

          return c.body(imageBuffer)
        } catch (fileError) {
          throw new HTTPException(404, { message: 'Thumbnail file not found on disk' })
        }
      } catch (error) {
        if (error instanceof HTTPException) {
          throw error
        }
        handleServiceError(error, 'Get task iteration thumbnail')
      }
    },
  )
}
