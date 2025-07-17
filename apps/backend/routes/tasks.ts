import {
  CloseTaskResponseSchema,
  CommandExecutionSchema,
  CommandResultSchema,
  CreateTaskRequestSchema,
  CreateVibeTaskRequestSchema,
  DeleteTaskResponseSchema,
  type EventListResponse,
  EventListResponseSchema,
  type EventType,
  EventTypeQuerySchema,
  FileContentResponseSchema,
  FileContentSchema,
  FileListResponseSchema,
  FileQuerySchema,
  FileSearchResponseSchema,
  FileWriteResponseSchema,
  GitDiffResponseSchema,
  GitStatusSchema,
  MergeTaskResponseSchema,
  MergeTaskSchema,
  MessagesResponseSchema,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  ServerStatusSchema,
  ServerStopResponseSchema,
  SimpleTaskResponseSchema,
  StopAgentResponseSchema,
  TaskCommandSchema,
  TaskCommandsResponseSchema,
  TaskFileSearchSchema,
  TaskListQuerySchema,
  type TaskListResponse,
  TaskListResponseSchema,
  TaskNotFoundError,
  TaskResponseSchema,
  ValidationError,
  WorkspaceError,
} from '@moatless/types'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import type { AppBindings } from '../app.js'
import { createMessageService } from '../services/message.service.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import { extractFilePathFromWildcard, validateRequestBody } from '../utils/route-helpers.js'

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
      const { model, filePaths, ...taskData } = validatedBody

      try {
        const taskId = await taskService.createVibeTask({ ...taskData, model, filePaths })

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
            enum: ['open', 'running', 'error', 'merged', 'closed'],
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
      const { message, resume, model, fileIds, filePaths, commandName } = validatedBody

      await taskService.sendMessage(taskId, message, resume, model, fileIds, filePaths, commandName)
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
      const { commandName, resume, model, filePaths } = body

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
        task,
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
      const { task: taskService, event: eventService } = services

      const taskId = getTaskId(c)

      // Verify task exists
      const task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // Only support Claude Code tasks with direct directory reading
      if (task.agentType !== 'claude-code') {
        throw new HTTPException(400, {
          message: 'Only Claude Code tasks are supported for message reading',
        })
      }

      // Get ClaudeCodeSettings from event service
      const claudeCodeSettings = (eventService as any).claudeCodeSettings
      if (!claudeCodeSettings) {
        throw new HTTPException(500, {
          message: 'ClaudeCodeSettings not available',
        })
      }

      const messageService = createMessageService(claudeCodeSettings)
      const messages = await messageService.readMessagesFromClaudeDirectory(task)

      return c.json({
        messages,
        totalCount: messages.length,
      })
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

      // Stop agent through AgentService
      const stopped = await services.agent.stopAgent(taskId)

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

  // List files in workspace
  app.get(
    `${prefix}/:taskId/files`,
    describeRoute({
      description: 'List files in task workspace',
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
          name: 'path',
          in: 'query',
          required: false,
          description: 'Directory path to list files from',
          schema: {
            type: 'string',
            default: '/',
          },
        },
        {
          name: 'recursive',
          in: 'query',
          required: false,
          description: 'List files recursively',
          schema: {
            type: 'boolean',
            default: false,
          },
        },
      ],
      responses: {
        200: {
          description: 'List of files',
          content: {
            'application/json': {
              schema: resolver(FileListResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task or path not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')
      const taskId = getTaskId(c)

      const queryParams = c.req.query()
      const validatedQuery = FileQuerySchema.parse(queryParams)
      const { path = '/', recursive = false } = validatedQuery

      try {
        // Task validation and capability check now handled in workspace service
        const workspace = await services.workspace.getOrCreateWorkspace(taskId)
        const filePaths = await workspace.listFiles(path, recursive)
        // Transform string paths to match the schema
        const files = filePaths.map((filePath) => ({
          name: filePath.split('/').pop() || filePath,
          path: filePath,
          type: filePath.endsWith('/') ? 'directory' : ('file' as const),
          size: undefined, // Size not available from listFiles
        }))
        return c.json({ files, totalCount: files.length })
      } catch (error) {
        handleServiceError(error, 'List files')
      }
    },
  )

  // Read file content
  app.get(
    `${prefix}/:taskId/files/*`,
    describeRoute({
      description: 'Read file content from task workspace',
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
          name: '*',
          in: 'path',
          required: true,
          description: 'File path',
          schema: {
            type: 'string',
          },
        },
      ],
      responses: {
        200: {
          description: 'File content',
          content: {
            'application/json': {
              schema: resolver(FileContentResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task or file not found',
        },
        413: {
          description: 'File too large',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')

      const taskId = getTaskId(c)

      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      try {
        // Task validation and capability check now handled in workspace service
        const workspace = await services.workspace.getOrCreateWorkspace(taskId)
        const content = await workspace.readFile(filePath)
        return c.json({ content, encoding: 'utf-8' })
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: 'Task not found' })
        }
        if (error instanceof Error && error.message.includes('File not found')) {
          throw new HTTPException(404, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('Access denied')) {
          throw new HTTPException(403, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('File too large')) {
          throw new HTTPException(413, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('Path is not a file')) {
          throw new HTTPException(400, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot read file')) {
          throw new HTTPException(403, { message: error.message })
        }
        throw error
      }
    },
  )

  // Write file content
  app.put(
    `${prefix}/:taskId/files/*`,
    describeRoute({
      description: 'Write file content to task workspace',
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
          name: '*',
          in: 'path',
          required: true,
          description: 'File path',
          schema: {
            type: 'string',
          },
        },
      ],
      requestBody: {
        description: 'File content',
        content: {
          'application/json': {
            schema: resolver(FileContentSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'File written successfully',
          content: {
            'application/json': {
              schema: resolver(FileWriteResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid request',
        },
        403: {
          description: 'Access denied or operation not allowed',
        },
        404: {
          description: 'Task not found',
        },
      },
    }),
    async (c) => {
      const services = c.get('services')

      const taskId = getTaskId(c)

      const filePath = extractFilePathFromWildcard(c.req.path, prefix, taskId)

      const validatedBody = await validateRequestBody(c, FileContentSchema)
      const { content } = validatedBody

      // Task validation and capability check now handled in workspace service
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      await workspace.writeFile(filePath, content)
      return c.json({ success: true })
    },
  )

  // Search files by filename/path
  app.post(
    `${prefix}/:taskId/search-files`,
    describeRoute({
      description: 'Search files by filename/path in task workspace',
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
        description: 'Search parameters',
        content: {
          'application/json': {
            schema: resolver(TaskFileSearchSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Search results',
          content: {
            'application/json': {
              schema: resolver(FileSearchResponseSchema) as any,
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

      const body = await c.req.json()
      const validatedBody = TaskFileSearchSchema.parse(body)
      const { query, limit } = validatedBody

      try {
        // Task validation and capability check now handled in workspace service
        const workspace = await services.workspace.getOrCreateWorkspace(taskId)
        const result = await workspace.searchFiles({
          query,
          limit,
        })
        return c.json(result)
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: 'Task not found' })
        }
        if (error instanceof Error && error.message.includes('Cannot search files')) {
          throw new HTTPException(403, { message: error.message })
        }
        throw error
      }
    },
  )

  // Get git diff for workspace
  app.get(
    `${prefix}/:taskId/git-diff`,
    describeRoute({
      description: 'Get git diff for task workspace',
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
          description: 'Git diff',
          content: {
            'application/json': {
              schema: resolver(GitDiffResponseSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Cannot access git',
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
        // Task validation and capability check now handled in workspace service
        const workspace = await services.workspace.getOrCreateWorkspace(taskId)
        const diffResult = await workspace.getGitDiff()

        return c.json({
          diff: diffResult.diffString,
          files: diffResult.files.map((f) => f.filename),
          hasChanges: diffResult.files.length > 0,
          // Include the detailed response as additional data
          detailed: diffResult,
        })
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: 'Task not found' })
        }
        if (error instanceof WorkspaceError) {
          throw new HTTPException(error.statusCode as any, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot access git')) {
          throw new HTTPException(403, { message: error.message })
        }
        throw error
      }
    },
  )

  // Get git sync status with origin for workspace
  app.get(
    `${prefix}/:taskId/git-sync-status`,
    describeRoute({
      description: 'Get git sync status with origin for task workspace',
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
          description: 'Git sync status',
          content: {
            'application/json': {
              schema: resolver(GitStatusSchema) as any,
            },
          },
        },
        400: {
          description: 'Invalid task ID',
        },
        403: {
          description: 'Cannot access git',
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
        // Get workspace status which now includes unified git status
        const workspaceStatus = await services.workspace.getWorkspaceStatus(taskId)

        if ((workspaceStatus as any).gitStatus) {
          return c.json((workspaceStatus as any).gitStatus)
        }
        // If git status is not available, return a default response
        return c.json({
          currentBranch: 'unknown',
          baseBranch: 'main',
          hasUncommittedChanges: false,
          hasConflicts: false,
          status: 'error',
          commitsAhead: 0,
          commitsBehind: 0,
          originExists: false,
        })
      } catch (error) {
        if (error instanceof TaskNotFoundError) {
          throw new HTTPException(404, { message: 'Task not found' })
        }
        if (error instanceof WorkspaceError) {
          throw new HTTPException(error.statusCode as any, { message: error.message })
        }
        if (error instanceof Error && error.message.includes('Cannot access git')) {
          throw new HTTPException(403, { message: error.message })
        }
        throw error
      }
    },
  )

  // Execute command in workspace
  app.post(
    `${prefix}/:taskId/execute`,
    describeRoute({
      description: 'Execute a command in task workspace',
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
        description: 'Command execution parameters',
        content: {
          'application/json': {
            schema: resolver(CommandExecutionSchema) as any,
          },
        },
      },
      responses: {
        200: {
          description: 'Command execution result',
          content: {
            'application/json': {
              schema: resolver(CommandResultSchema) as any,
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

      const body = await c.req.json()
      const validatedBody = CommandExecutionSchema.parse(body)
      const { command, timeout } = validatedBody

      // Task validation and capability check now handled in workspace service
      const workspace = await services.workspace.getOrCreateWorkspace(taskId)
      const result = await workspace.runCommand(command, timeout)
      return c.json(result)
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
}
