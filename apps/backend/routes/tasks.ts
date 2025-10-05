import type { OpenAPIHono } from '@hono/zod-openapi'
import { createRoute } from '@hono/zod-openapi'
import {
  AgentMessagesResponseSchema,
  CloseTaskResponseSchemaAPI,
  CommandExecutionOptionsSchema,
  CreateTaskRequestSchema,
  CreateVibeTaskRequestSchema,
  DeleteTaskResponseSchema,
  DeployResponseSchema,
  McpResponseSchema,
  SendMessageRequestSchema,
  SendMessageResponseSchema,
  ServerLogsQuerySchema,
  ServerLogsResponseExtendedSchema,
  SimpleTaskResponseSchema,
  StartTaskResponseSchema,
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
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import { logger } from '../utils/logger.js'
import {
  createGetByIdRoute,
  createGetListRoute,
  createPostRoute,
  pathParams,
  withStandardErrors,
} from '../utils/openapi-helpers.js'
import { readTaskLog, taskLogExists } from '../utils/task-log.js'

export async function registerTaskRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Create a new task
  const createTaskRoute = createPostRoute(
    '/api/v1/tasks',
    CreateTaskRequestSchema,
    SimpleTaskResponseSchema,
    {
      operation: 'Create a new AI agent task to make code changes in a repository',
      request:
        'Provide task configuration including agent type, repository ID, optional initial message, branch settings, and execution preferences. Set start=true to begin AI agent execution immediately after creation.',
      response:
        'Returns the created task with unique ID, current status (open/running), and complete task details including iterations and workspace path',
    },
    200, // Use 200 instead of 201 to match existing behavior
    ['Tasks'],
  )

  app.openapi(createTaskRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const body = c.req.valid('json')
    const { model, filePaths, ...taskData } = body

    try {
      const task = await taskService.createTask({ ...taskData, model, filePaths })
      return c.json({ taskId: task.taskId, status: task.status, task })
    } catch (error) {
      handleServiceError(error, 'Create task')
    }
  })

  // Create a new vibe task from template
  const createVibeTaskRoute = createPostRoute(
    '/api/v1/tasks/vibe',
    CreateVibeTaskRequestSchema,
    SimpleTaskResponseSchema,
    {
      operation: 'Create a new vibe task from a predefined template for quick prototyping',
      request:
        'Provide template ID, agent type, optional initial message, and file uploads. Vibe tasks use temporary workspaces and start execution immediately.',
      response:
        'Returns the created vibe task with unique ID, running status, and task details. The task begins executing immediately after creation.',
    },
    200,
    ['Tasks'],
  )

  app.openapi(createVibeTaskRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const body = c.req.valid('json')
    const { filePaths, ...taskData } = body

    try {
      const taskId = await taskService.createVibeTask({ ...taskData, filePaths })

      // Get the task to return the current status
      const task = await taskService.getTask(taskId)
      return c.json({ taskId, status: task?.status || 'open', task })
    } catch (error) {
      handleServiceError(error, 'Create vibe task')
    }
  })

  // Get task by ID
  const getTaskRoute = createGetByIdRoute(
    '/api/v1/tasks/{taskId}',
    pathParams.taskId,
    TaskResponseSchema,
    {
      operation: 'Retrieve a specific task by its unique identifier',
      response:
        'Returns complete task details including all iterations, current status, cost tracking, workspace path, and git information',
    },
    ['Tasks'],
  )

  app.openapi(getTaskRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    const { taskId } = c.req.valid('param')

    try {
      const task = await taskService.getTask(taskId)
      return c.json(task)
    } catch (error) {
      handleServiceError(error, 'Get task')
    }
  })

  // List all tasks with pagination
  const listTasksRoute = createGetListRoute(
    '/api/v1/tasks',
    TaskListQuerySchema,
    TaskListResponseSchema,
    {
      operation: 'List all tasks with optional pagination and status filtering',
      response:
        'Returns paginated list of tasks with pagination metadata (limit, offset, total, hasMore). Each task includes iterations, status, and cost information.',
    },
    ['Tasks'],
  )

  app.openapi(listTasksRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { limit, offset, status } = c.req.valid('query')

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
  })

  // Delete task (custom route that returns 200 with body, not 204 No Content)
  const deleteTaskRoute = createRoute({
    method: 'delete',
    path: '/api/v1/tasks/{taskId}',
    tags: ['Tasks'],
    description:
      'Delete a task and clean up associated resources including workspace, git worktree, and event data',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      DeleteTaskResponseSchema,
      'Returns confirmation that the task was deleted. All associated workspace files, git worktree, and task data are removed.',
    ),
  })

  app.openapi(deleteTaskRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services
    const { taskId } = c.req.valid('param')

    try {
      await taskService.deleteTask(taskId)
      return c.json({ status: 'deleted' })
    } catch (error) {
      handleServiceError(error, 'Delete task')
    }
  })

  // Send message to task
  const sendMessageRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/messages',
    tags: ['Task Messages'],
    description:
      'Send a message to an AI agent task to provide additional instructions or feedback',
    request: {
      params: pathParams.taskId,
      body: {
        content: {
          'application/json': {
            schema: SendMessageRequestSchema,
          },
        },
        description:
          'Provide a message string, optional model override, file attachments, command name, and execution preferences. Set resume=true (default) to continue agent execution after sending the message.',
      },
    },
    responses: withStandardErrors(
      SendMessageResponseSchema,
      'Returns confirmation that the message was sent successfully with a timestamp',
    ),
  })

  app.openapi(sendMessageRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')
    const { message, resume, model, fileIds, filePaths, commandName, agentMode } =
      c.req.valid('json')

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
  })

  // Get available commands for task
  const getCommandsRoute = createRoute({
    method: 'get',
    path: '/api/v1/tasks/{taskId}/commands',
    tags: ['Task Commands'],
    description: 'Retrieve list of available AI agent commands for a task based on current state',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      TaskCommandsResponseSchema,
      'Returns array of available commands with names, descriptions, prompts, and execution conditions',
    ),
  })

  app.openapi(getCommandsRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')

    const commands = await taskService.getAvailableCommands(taskId)
    return c.json({ commands })
  })

  // Send command to task
  const sendCommandRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/commands',
    tags: ['Task Commands'],
    description: 'Execute a predefined command on an AI agent task to perform specific actions',
    request: {
      params: pathParams.taskId,
      body: {
        content: {
          'application/json': {
            schema: TaskCommandSchema,
          },
        },
        description:
          'Provide command name, optional model override, file paths, and execution preferences. Set resume=true (default) to continue agent execution after the command.',
      },
    },
    responses: withStandardErrors(
      SendMessageResponseSchema,
      'Returns confirmation that the command was sent successfully with a timestamp',
    ),
  })

  app.openapi(sendCommandRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')
    const { commandName, resume, model, filePaths } = c.req.valid('json')

    await taskService.sendCommand(taskId, commandName, resume, model, filePaths)
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get task messages with merged tool calls
  const getMessagesRoute = createRoute({
    method: 'get',
    path: '/api/v1/tasks/{taskId}/messages',
    tags: ['Task Messages'],
    description: 'Retrieve all messages for a task with tool calls merged into assistant messages',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      AgentMessagesResponseSchema,
      'Returns array of messages (user and assistant) with total count. Tool calls are merged into assistant messages for cleaner display.',
    ),
  })

  app.openapi(getMessagesRoute, async (c) => {
    const services = c.get('services')

    const { taskId } = c.req.valid('param')

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
  })

  // Start task
  const startTaskRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/start',
    tags: ['Task Execution'],
    description: 'Start AI agent execution for a task that was created without auto-start',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      StartTaskResponseSchema,
      'Returns confirmation that the task has started with task ID and status',
    ),
  })

  app.openapi(startTaskRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

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
  })

  // Stop agent
  const stopAgentRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/stop',
    tags: ['Task Execution'],
    description:
      'Stop the running AI agent for a task while preserving the workspace and task data',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      StopAgentResponseSchema,
      'Returns stop status (stopped or not_running) and task ID',
    ),
  })

  app.openapi(stopAgentRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

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
  })

  // Close task (teardown workspace, stop server, set status to closed)
  const closeTaskRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/close',
    tags: ['Task Execution'],
    description:
      'Close a task and tear down its workspace, stop dev server, and mark status as closed',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      CloseTaskResponseSchemaAPI,
      'Returns confirmation that the task was closed with status and task ID',
    ),
  })

  app.openapi(closeTaskRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')

    await taskService.closeTask(taskId)

    return c.json({
      status: 'closed',
      taskId,
    })
  })

  // Execute commands in task workspace with streaming output
  // Note: This route uses Server-Sent Events (SSE) streaming and is not migrated to OpenAPI
  // because SSE streaming doesn't fit well with the standard OpenAPI JSON response pattern
  app.post(`${prefix}/:taskId/execute-commands`, async (c) => {
    const services = c.get('services')
    const { workspace: workspaceService } = services

    const taskId = getTaskId(c)

    const body = await c.req.json()
    const options = CommandExecutionOptionsSchema.parse(body)

    // Import SSE utility
    const { createSSEStream } = await import('../utils/sse-stream.js')

    // Create SSE stream from command execution generator
    const shellLogGenerator = workspaceService.executeCommandsInTaskWorkspace(taskId, options)

    // Transform ShellCommandLogEvent to SSEEvent format
    async function* transformToSSEEvents() {
      for await (const event of shellLogGenerator) {
        yield {
          type: 'shell-log',
          data: {
            logType: event.type,
            timestamp: event.timestamp,
            text: event.text,
          },
        }
      }
    }

    return createSSEStream(transformToSSEEvents(), {
      headers: {
        'Cache-Control': 'no-cache',
        'X-Task-Id': taskId,
      },
    })
  })

  // Get task logs (setup commands, server logs, all workspace operations)
  const getTaskLogsRoute = createRoute({
    method: 'get',
    path: '/api/v1/tasks/{taskId}/logs',
    tags: ['Task Logs'],
    description:
      'Retrieve complete task logs including setup commands, server output, and all workspace operations',
    request: {
      params: pathParams.taskId,
      query: ServerLogsQuerySchema,
    },
    responses: withStandardErrors(
      ServerLogsResponseExtendedSchema,
      'Returns task logs with line count and truncation status. Returns empty logs if log file does not exist.',
    ),
  })

  app.openapi(getTaskLogsRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

    // Validate task exists
    const task = await services.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Parse query parameters
    const queryParams = c.req.query()
    const validatedQuery = ServerLogsQuerySchema.parse({
      lines: queryParams.lines ? Number.parseInt(queryParams.lines) : undefined,
    })
    const { lines } = validatedQuery

    // Check if log file exists
    const logExists = await taskLogExists(taskId)
    if (!logExists) {
      return c.json({
        logs: '',
        lines: 0,
        truncated: false,
      })
    }

    // Read task logs
    const logs = await readTaskLog(taskId, lines)
    return c.json({
      logs,
      lines: logs.split('\n').filter((line) => line.trim() !== '').length,
      truncated: lines !== undefined && lines > 0,
    })
  })

  // MCP (Model Context Protocol) endpoint for task-scoped tools
  const mcpRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/mcp',
    tags: ['Task MCP'],
    description:
      'Execute Model Context Protocol (MCP) JSON-RPC requests for task-scoped tools and operations',
    request: {
      params: pathParams.taskId,
      body: {
        content: {
          'application/json': {
            schema: McpResponseSchema,
          },
        },
        description:
          'JSON-RPC 2.0 request body containing method, params, and optional id field for MCP protocol communication',
      },
    },
    responses: withStandardErrors(
      McpResponseSchema,
      'Returns JSON-RPC 2.0 response with result or error object',
    ),
  })

  app.openapi(mcpRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')

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
  })

  // Deploy task to Netlify
  const deployRoute = createRoute({
    method: 'post',
    path: '/api/v1/tasks/{taskId}/deploy',
    tags: ['Task Deployment'],
    description: 'Deploy task workspace to Netlify for live preview and sharing',
    request: { params: pathParams.taskId },
    responses: withStandardErrors(
      DeployResponseSchema,
      'Returns deployment result with success status, site URL, deploy URL, site ID, and deployment metadata. Updates task with site information on successful deployment.',
    ),
  })

  app.openapi(deployRoute, async (c) => {
    const services = c.get('services')
    const { taskId } = c.req.valid('param')
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
  })

  // Get task iteration thumbnail
  // Note: This route returns binary image data (image/png) and is not migrated to OpenAPI
  // because binary responses don't fit well with the standard OpenAPI JSON response pattern
  app.get(`${prefix}/:taskId/iterations/:iterationNumber/thumbnail`, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const taskId = getTaskId(c)
    const iterationNumber = Number.parseInt(c.req.param('iterationNumber'), 10)

    if (Number.isNaN(iterationNumber)) {
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
      } catch (_fileError) {
        throw new HTTPException(404, { message: 'Thumbnail file not found on disk' })
      }
    } catch (error) {
      if (error instanceof HTTPException) {
        throw error
      }
      handleServiceError(error, 'Get task iteration thumbnail')
    }
  })
}
