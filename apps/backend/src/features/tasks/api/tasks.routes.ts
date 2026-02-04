import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  closeTaskRoute,
  createTaskRoute,
  createVibeTaskRoute,
  deleteTaskRoute,
  getCommandsRoute,
  getTaskRoute,
  listTasksRoute,
  sendCommandRoute,
  sendMessageRoute,
  startTaskRoute,
  stopAgentRoute,
  updateTaskMetadataRoute,
  updateTaskTagsRoute,
  updateTaskVisibilityRoute,
} from '@moatless/api-contract/tasks'
import type { AskUserQuestionResponse, ExitPlanModeResponse, Task } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { AgentControlService } from '../../agents/services/agent-control.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskService } from '../services/task.service'

/**
 * Serialize a Task for JSON response, converting Date fields to ISO strings.
 * Hono's OpenAPI type system expects string dates matching the Zod schema's
 * OpenAPI `type: 'string', format: 'date-time'` metadata.
 */
function serializeTask(task: Task) {
  return {
    ...task,
    createdAt: task.createdAt.toISOString(),
    lastMessageAt: task.lastMessageAt.toISOString(),
  }
}

export function registerTaskRoutes(app: OpenAPIHono<AppBindings>) {
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
      return c.json({ taskId: task.taskId, status: task.status, task: serializeTask(task) })
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
      return c.json({ taskId, status: task?.status || 'open', task: serializeTask(task) })
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
      return c.json(serializeTask(task))
    } catch (error) {
      handleServiceError(error, 'Get task')
    }
  })

  // List all tasks with pagination
  app.openapi(listTasksRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { limit, offset, status, tag, createdBy, repositoryId } = c.req.valid('query')

    const filters: { tag?: string; createdBy?: string; repositoryId?: string } = {}
    if (tag) {
      filters.tag = tag
    }
    if (createdBy) {
      filters.createdBy = createdBy
    }
    if (repositoryId) {
      filters.repositoryId = repositoryId
    }

    const { tasks, total } = await taskService.listTasks(limit, offset, status, filters)

    const response = {
      items: tasks.map(serializeTask),
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

  // Update task visibility
  app.openapi(updateTaskVisibilityRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { visibility } = c.req.valid('json')

    try {
      const task = await taskService.getTask(taskId)
      task.visibility = visibility
      await taskService.updateTask(taskId, task)
      return c.json({ success: true as const })
    } catch (error) {
      handleServiceError(error, 'Update task visibility')
    }
  })

  // Update task metadata (name and description)
  app.openapi(updateTaskMetadataRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { name, description } = c.req.valid('json')

    try {
      // Build updates object with only defined values
      const updates: { name?: string; description?: string } = {}
      if (name !== undefined) {
        updates.name = name
      }
      if (description !== undefined) {
        updates.description = description
      }

      await taskService.updateTaskWithEvent(taskId, updates)
      return c.json({ success: true as const })
    } catch (error) {
      handleServiceError(error, 'Update task metadata')
    }
  })

  // Update task tags
  app.openapi(updateTaskTagsRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const { taskId } = c.req.valid('param')
    const { tags } = c.req.valid('json')

    try {
      const resolvedTags = await taskService.setTaskTags(taskId, tags)
      return c.json({ tags: resolvedTags })
    } catch (error) {
      handleServiceError(error, 'Update task tags')
    }
  })

  // Send message to task (also handles user input responses for interactive tools)
  app.openapi(sendMessageRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const body = c.req.valid('json')
    const { message, resume, model, fileIds, contexts, commandName, agentMode } = body

    // Tool response fields for interactive tools (AskUserQuestion, ExitPlanMode).
    // Accessed via indexing because Hono's type inference doesn't resolve all
    // properties from complex ZodObject schemas.
    const rawBody = body as Record<string, unknown>
    const toolUseId = rawBody['toolUseId'] as string | undefined
    const toolName = rawBody['toolName'] as 'AskUserQuestion' | 'ExitPlanMode' | undefined
    const toolResponse = rawBody['toolResponse'] as
      | AskUserQuestionResponse
      | ExitPlanModeResponse
      | undefined

    await taskService.sendMessage(
      taskId,
      message,
      resume,
      model,
      fileIds,
      commandName,
      agentMode,
      contexts,
      toolUseId,
      toolName,
      toolResponse,
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
    const agentControlService = appContainer.get(AgentControlService)
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      // Check if agent is currently running
      const wasRunning = await runtimeManager.isAgentActuallyRunning(taskId)

      // Stop agent (no-op if not running)
      await agentControlService.stopAgent(taskId)

      return c.json({
        status: wasRunning ? 'stopped' : 'not_running',
        taskId,
      })
    } catch (error) {
      handleServiceError(error, 'Stop agent')
    }
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
}
