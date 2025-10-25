import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  closeTaskRoute,
  createTaskRoute,
  createVibeTaskRoute,
  deleteTaskRoute,
  deployRoute,
  getCommandsRoute,
  getMessagesRoute,
  getTaskRoute,
  listTasksRoute,
  mcpRoute,
  sendCommandRoute,
  sendMessageRoute,
  startTaskRoute,
  stopAgentRoute,
} from '@moatless/api-contract/tasks'
import type { DeployResponse, PublishSettings, Task, TaskListResponse } from '@moatless/types'
import { CommandExecutionOptionsSchema, TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { appContainer } from '../container/container.js'
import { McpService } from '../services/mcp.service.js'
import { MessageService } from '../services/message.service.js'
import { SettingsService } from '../services/settings.service.js'
import { TaskService } from '../services/task.service.js'
import { WorkspaceService } from '../services/workspace.service.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import { logger } from '../utils/logger.js'

type DeploymentInfo = Pick<DeployResponse, 'success' | 'siteId' | 'siteUrl'>

export function registerTaskRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Create a new task
  app.openapi(createTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const body = c.req.valid('json')
    const { model, fileIds, ...taskData } = body

    try {
      const task = await taskService.createTask({
        ...taskData,
        model,
        fileIds,
      })
      return c.json({ taskId: task.taskId, status: task.status, task })
    } catch (error) {
      handleServiceError(error, 'Create task')
    }
  })

  // Create a new vibe task from template
  app.openapi(createVibeTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const body = c.req.valid('json')
    const { fileIds, ...taskData } = body

    try {
      const taskId = await taskService.createVibeTask({
        ...taskData,
        fileIds,
      })

      // Get the task to return the current status
      const task = await taskService.getTask(taskId)
      return c.json({ taskId, status: task?.status || 'open', task })
    } catch (error) {
      handleServiceError(error, 'Create vibe task')
    }
  })

  // Get task by ID
  app.openapi(getTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      const task = await taskService.getTask(taskId)
      return c.json(task)
    } catch (error) {
      handleServiceError(error, 'Get task')
    }
  })

  // List all tasks with pagination
  app.openapi(listTasksRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

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
  app.openapi(deleteTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      await taskService.deleteTask(taskId)
      return c.json({ status: 'deleted' })
    } catch (error) {
      handleServiceError(error, 'Delete task')
    }
  })

  // Send message to task
  app.openapi(sendMessageRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const { message, resume, model, fileIds, contexts, commandName, agentMode } =
      c.req.valid('json')

    await taskService.sendMessage(
      taskId,
      message,
      resume,
      model,
      fileIds,
      commandName,
      agentMode,
      contexts,
    )
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get available commands for task
  app.openapi(getCommandsRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')

    const commands = await taskService.getAvailableCommands(taskId)
    return c.json({ commands })
  })

  // Send command to task
  app.openapi(sendCommandRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const { commandName, resume, model, fileIds } = c.req.valid('json')

    await taskService.sendCommand(taskId, commandName, resume, model, fileIds)
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get task messages with merged tool calls
  app.openapi(getMessagesRoute, async (c) => {
    const messageService = appContainer.get(MessageService)

    const { taskId } = c.req.valid('param')

    try {
      const messages = await messageService.getTaskMessages(taskId)

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
  app.openapi(startTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')

    try {
      await taskService.startTask(taskId)
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
  app.openapi(stopAgentRoute, async (c) => {
    const workspaceService = appContainer.get(WorkspaceService)
    const { taskId } = c.req.valid('param')

    // Stop agent through workspace environment
    const workspace = await workspaceService.getWorkspace(taskId)
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
  app.openapi(closeTaskRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

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
    const workspaceService = appContainer.get(WorkspaceService)

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

  // MCP (Model Context Protocol) endpoint for task-scoped tools
  app.openapi(mcpRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const mcpService = appContainer.get(McpService)
    const { taskId } = c.req.valid('param')

    try {
      // Verify task exists
      await taskService.getTask(taskId)

      // Parse JSON-RPC request body
      const requestBody = await c.req.json()

      // Handle the request through MCP service
      const response = await mcpService.handleRequest(taskId, requestBody)

      return c.json(response as unknown as Record<string, unknown>)
    } catch (error) {
      logger.error('MCP request error:', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return handleServiceError(error, 'MCP request')
    }
  })

  /**
   * Update task with deployment information after successful deployment
   */
  async function updateTaskWithDeploymentInfo(
    taskId: string,
    result: DeploymentInfo,
  ): Promise<void> {
    if (!result.success) {
      return
    }

    try {
      const taskService = appContainer.get(TaskService)
      const updatedTask = await taskService.getTask(taskId)

      updateTaskSiteInfo(updatedTask, result)
      updateLatestIterationSiteUrl(updatedTask, result)

      await taskService.updateTask(taskId, updatedTask)

      logDeploymentUpdate(taskId, result, updatedTask)
    } catch (updateError) {
      logDeploymentUpdateError(taskId, updateError)
    }
  }

  function updateTaskSiteInfo(task: Task, result: Pick<DeploymentInfo, 'siteId'>): void {
    if (result.siteId && !task.siteId) {
      task.siteId = result.siteId
      task.siteProvider = 'netlify'
    }
  }

  function updateLatestIterationSiteUrl(task: Task, result: Pick<DeploymentInfo, 'siteUrl'>): void {
    if (!result.siteUrl || task.iterations.length === 0) {
      return
    }

    const latestIteration = task.iterations[task.iterations.length - 1]
    if (latestIteration) {
      latestIteration.siteUrl = result.siteUrl
    }
  }

  function logDeploymentUpdate(
    taskId: string,
    result: Pick<DeploymentInfo, 'siteId' | 'siteUrl'>,
    updatedTask: Task,
  ): void {
    logger.info('Updated task with deployment info', {
      taskId,
      siteId: result.siteId,
      siteUrl: result.siteUrl,
      iterationNumber:
        updatedTask.iterations.length > 0
          ? updatedTask.iterations[updatedTask.iterations.length - 1]?.iterationNumber
          : undefined,
    })
  }

  function logDeploymentUpdateError(taskId: string, error: unknown): void {
    logger.warn('Failed to update task with deployment info', {
      taskId,
      error: error instanceof Error ? error.message : String(error),
    })
  }

  function handleDeploymentError(
    error: unknown,
    taskId: string,
    task: Task | undefined,
  ): DeployResponse {
    if (error instanceof HTTPException) {
      throw error
    }
    if (error instanceof TaskNotFoundError) {
      throw new HTTPException(404, { message: error.message })
    }

    const errorMessage = error instanceof Error ? error.message : 'Unknown deployment error'
    logger.error('Deployment failed', error as Error, {
      taskId,
      taskName: task?.name,
      error: errorMessage,
    })

    return {
      success: false,
      error: errorMessage,
      logs: `Deployment failed: ${errorMessage}`,
    }
  }

  function validateNetlifyToken(publishSettings: Required<PublishSettings>): void {
    if (!publishSettings.netlifyAccessToken) {
      throw new HTTPException(400, {
        message:
          'Netlify access token not configured in settings. Please configure it in Settings > Publish.',
      })
    }
  }

  // Deploy task to Netlify
  app.openapi(deployRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const settingsService = appContainer.get(SettingsService)
    const workspaceService = appContainer.get(WorkspaceService)
    const { taskId } = c.req.valid('param')
    let task: Task | null = null

    try {
      task = await taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      const publishSettings = settingsService.getPublishSettings()
      validateNetlifyToken(publishSettings)

      const netlifyAccessToken = publishSettings.netlifyAccessToken
      if (!netlifyAccessToken) {
        throw new HTTPException(500, { message: 'Netlify access token validation failed' })
      }

      const result = await workspaceService.deployToNetlify(
        taskId,
        task.name,
        netlifyAccessToken,
        task.siteId || undefined,
      )

      const deploymentInfo: DeploymentInfo = { success: result.success }
      if (result.siteId) {
        deploymentInfo.siteId = result.siteId
      }
      if (result.siteUrl) {
        deploymentInfo.siteUrl = result.siteUrl
      }
      await updateTaskWithDeploymentInfo(taskId, deploymentInfo)

      return c.json(result)
    } catch (error) {
      return c.json(handleDeploymentError(error, taskId, task ?? undefined))
    }
  })

  // Get task iteration thumbnail
  // Note: This route returns binary image data (image/png) and is not migrated to OpenAPI
  // because binary responses don't fit well with the standard OpenAPI JSON response pattern
  app.get(`${prefix}/:taskId/iterations/:iterationNumber/thumbnail`, async (c) => {
    const { ScreenshotService } = await import('../services/screenshot.service.js')
    const screenshotService = appContainer.get(ScreenshotService)
    const taskId = getTaskId(c)
    const iterationNumber = Number.parseInt(c.req.param('iterationNumber'), 10)

    if (Number.isNaN(iterationNumber)) {
      throw new HTTPException(404, { message: 'Invalid iteration number' })
    }

    try {
      const imageBuffer = await screenshotService.getIterationImage(
        taskId,
        iterationNumber,
        'thumbnail',
      )

      // Set headers for immutable image response with aggressive caching
      c.header('Content-Type', 'image/png')
      c.header('Cache-Control', 'public, max-age=31536000, immutable') // 1 year, immutable

      return c.body(new Uint8Array(imageBuffer))
    } catch (error) {
      if (error instanceof HTTPException) {
        throw error
      }
      handleServiceError(error, 'Get task iteration thumbnail')
    }
  })

  // Get task iteration screenshot (full-size)
  // Note: This route returns binary image data (image/png) and is not migrated to OpenAPI
  // because binary responses don't fit well with the standard OpenAPI JSON response pattern
  app.get(`${prefix}/:taskId/iterations/:iterationNumber/screenshot`, async (c) => {
    const { ScreenshotService } = await import('../services/screenshot.service.js')
    const screenshotService = appContainer.get(ScreenshotService)
    const taskId = getTaskId(c)
    const iterationNumber = Number.parseInt(c.req.param('iterationNumber'), 10)

    if (Number.isNaN(iterationNumber)) {
      throw new HTTPException(404, { message: 'Invalid iteration number' })
    }

    try {
      const imageBuffer = await screenshotService.getIterationImage(
        taskId,
        iterationNumber,
        'screenshot',
      )

      // Set headers for immutable image response with aggressive caching
      c.header('Content-Type', 'image/png')
      c.header('Cache-Control', 'public, max-age=31536000, immutable') // 1 year, immutable

      return c.body(new Uint8Array(imageBuffer))
    } catch (error) {
      if (error instanceof HTTPException) {
        throw error
      }
      handleServiceError(error, 'Get task iteration screenshot')
    }
  })
}
