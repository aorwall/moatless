import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  closeTaskRoute,
  createTaskRoute,
  createVibeTaskRoute,
  deleteTaskRoute,
  deployRoute,
  getCommandsRoute,
  getMessagesRoute,
  getTaskLogsRoute,
  getTaskRoute,
  listTasksRoute,
  mcpRoute,
  sendCommandRoute,
  sendMessageRoute,
  startTaskRoute,
  stopAgentRoute,
} from '@moatless/api-contract/tasks'
import {
  CommandExecutionOptionsSchema,
  ServerLogsQuerySchema,
  type TaskListResponse,
  TaskNotFoundError,
  ValidationError,
} from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { getTaskId, handleServiceError } from '../utils/error-handler.js'
import { logger } from '../utils/logger.js'
import { readTaskLog, taskLogExists } from '../utils/task-log.js'

export async function registerTaskRoutes(app: OpenAPIHono<AppBindings>, prefix: string) {
  // Create a new task
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
  app.openapi(getCommandsRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')

    const commands = await taskService.getAvailableCommands(taskId)
    return c.json({ commands })
  })

  // Send command to task
  app.openapi(sendCommandRoute, async (c) => {
    const services = c.get('services')
    const { task: taskService } = services

    const { taskId } = c.req.valid('param')
    const { commandName, resume, model, filePaths } = c.req.valid('json')

    await taskService.sendCommand(taskId, commandName, resume, model, filePaths)
    return c.json({ status: 'sent', timestamp: new Date().toISOString() })
  })

  // Get task messages with merged tool calls
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

      return c.json(response as unknown as Record<string, unknown>)
    } catch (error) {
      console.error('MCP request error:', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return handleServiceError(error, 'MCP request')
    }
  })

  // Deploy task to Netlify
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
