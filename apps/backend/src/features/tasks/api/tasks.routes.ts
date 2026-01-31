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
  userInputResponseRoute,
} from '@moatless/api-contract/tasks'
import type { Task, TaskListResponse } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { handleServiceError } from '../../../utils/error-handler'
import { logger } from '../../../utils/logger'
import { AgentControlService } from '../../agents/services/agent-control.service'
import { MessageService } from '../../agents/services/message.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type {
  DockerExecutionTarget,
  ExecutionTarget,
  KubernetesExecutionTarget,
} from '../../workspace/types/workspace-runtime.types'
import { TaskRepository } from '../db/task.repository'
import { TaskService } from '../services/task.service'

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
      const snapshot = await runtimeManager.getSnapshot(taskId)
      const wasRunning = snapshot?.agent?.state === 'running'

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

  // Submit user input response (AskUserQuestion or ExitPlanMode)
  app.openapi(userInputResponseRoute, async (c) => {
    const { taskId } = c.req.valid('param')
    const body = c.req.valid('json')
    const { toolName, response } = body

    // 1. Save the response as a message for conversation history
    const messageService = appContainer.get(MessageService)
    try {
      await messageService.saveUserInputResponse(taskId, toolName, response)
    } catch (error) {
      logger.warn({ taskId, error }, 'Failed to save user input response, continuing anyway')
    }

    // Track plan approval status on iteration for ExitPlanMode
    if (toolName === 'ExitPlanMode') {
      try {
        const planResponse = response as { approved: boolean }
        const taskRepository = appContainer.get(TaskRepository)
        const task = await taskRepository.get(taskId)
        const currentIteration = task?.iterations[task.iterations.length - 1]
        if (currentIteration) {
          currentIteration.planApproved = planResponse.approved
          await taskRepository.save(task)
          logger.info(
            { taskId, planApproved: planResponse.approved },
            'Updated iteration with plan approval status',
          )
        }
      } catch (error) {
        logger.warn({ taskId, error }, 'Failed to track plan approval status')
      }
    }

    // 2. Ensure workspace is running and get sandbox URL
    const workspaceResult = await ensureWorkspaceForUserInput(taskId)
    if (!workspaceResult.ok) {
      return c.json({ success: false, error: workspaceResult.error }, workspaceResult.status)
    }

    // 3. Proxy to sandbox
    const proxyResult = await proxyUserInputToSandbox(workspaceResult.sandboxUrl, body, taskId)
    if (proxyResult.ok) {
      return c.json({ success: true }, 200)
    }

    // Handle sandbox state loss (404) - check if we can provide session ID for resume
    if (proxyResult.status === 404) {
      const agentSessionId = findResumableSession(workspaceResult.task)
      if (agentSessionId) {
        return c.json(
          {
            success: false,
            error: 'Agent session was interrupted. The session can be resumed.',
            canResume: true,
            agentSessionId,
          },
          409,
        )
      }
    }

    return c.json({ success: false, error: proxyResult.error }, proxyResult.status)
  })
}

/**
 * Get sandbox URL from execution target
 */
function getSandboxUrl(target: ExecutionTarget): string | null {
  if (target.mode === 'docker') {
    const dockerTarget = target as DockerExecutionTarget
    return `http://${dockerTarget.containerName}:8001`
  }
  if (target.mode === 'kubernetes') {
    const k8sTarget = target as KubernetesExecutionTarget
    if (!k8sTarget.podIp) {
      return null
    }
    return `http://${k8sTarget.podIp}:${k8sTarget.workerPort}`
  }
  return null
}

type UserInputProxyResult = { ok: true } | { ok: false; error: string; status: 400 | 404 | 503 }

/** Result of ensuring workspace is ready for user input */
type EnsureWorkspaceResult =
  | { ok: true; sandboxUrl: string; task: Task }
  | { ok: false; error: string; status: 404 | 503 }

/**
 * Ensure workspace is provisioned and ready for user input response
 */
async function ensureWorkspaceForUserInput(taskId: string): Promise<EnsureWorkspaceResult> {
  const workspaceCoordinator = appContainer.get(WorkspaceRuntimeManager)
  const taskService = appContainer.get(TaskService)

  // Get task to get repositoryId for ensureWorkspace
  let task: Task
  try {
    task = await taskService.getTask(taskId)
  } catch (error) {
    if (error instanceof TaskNotFoundError) {
      return { ok: false, error: 'Task not found', status: 404 }
    }
    throw error
  }

  // Ensure workspace is provisioned and running
  try {
    await workspaceCoordinator.ensureWorkspace(taskId, task.repositoryId)
  } catch (error) {
    logger.error({ taskId, error }, 'Failed to ensure workspace is running')
    return {
      ok: false,
      error: error instanceof Error ? error.message : 'Failed to start workspace',
      status: 503,
    }
  }

  // Get snapshot for sandbox URL
  const snapshot = await workspaceCoordinator.getSnapshot(taskId)
  if (!snapshot) {
    logger.warn({ taskId }, 'Workspace snapshot not found after ensureWorkspace')
    return { ok: false, error: 'Workspace not found', status: 503 }
  }

  const sandboxUrl = getSandboxUrl(snapshot.executionTarget)
  if (!sandboxUrl) {
    logger.warn(
      { taskId, executionTarget: snapshot.executionTarget },
      'Sandbox URL not available - podIp may be missing',
    )
    return { ok: false, error: 'Sandbox URL not available', status: 503 }
  }

  return { ok: true, sandboxUrl, task }
}

/**
 * Check if a task has a resumable session after sandbox returns 404
 * Returns the session ID if found, null otherwise
 */
function findResumableSession(task: Task): string | null {
  const activeIteration = task.iterations.find((it) => it.status === 'active' && it.agentSessionId)
  return activeIteration?.agentSessionId ?? null
}

/**
 * Proxy user input response to sandbox
 */
async function proxyUserInputToSandbox(
  sandboxUrl: string,
  body: unknown,
  taskId: string,
): Promise<UserInputProxyResult> {
  try {
    const proxyResponse = await fetch(`${sandboxUrl}/user-input-response`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })

    const result = (await proxyResponse.json()) as { success: boolean; error?: string }

    if (!proxyResponse.ok) {
      const error = result.error || 'Sandbox error'
      const status = proxyResponse.status
      if (status === 400) {
        return { ok: false, error, status: 400 }
      }
      if (status === 404) {
        return { ok: false, error, status: 404 }
      }
      return { ok: false, error, status: 503 }
    }

    return { ok: true }
  } catch (error) {
    logger.error({ taskId, error }, 'Failed to proxy user input response to sandbox')
    return {
      ok: false,
      error: error instanceof Error ? error.message : 'Failed to proxy to sandbox',
      status: 503,
    }
  }
}
