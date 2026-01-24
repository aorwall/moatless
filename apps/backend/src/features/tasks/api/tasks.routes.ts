import path from 'node:path'
import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  closeTaskRoute,
  createTaskRoute,
  createVibeTaskRoute,
  deleteTaskRoute,
  deployRoute,
  getCommandsRoute,
  getTaskRoute,
  listTasksRoute,
  mcpRoute,
  sendCommandRoute,
  sendMessageRoute,
  startTaskRoute,
  stopAgentRoute,
  updateTaskVisibilityRoute,
  userInputResponseRoute,
} from '@moatless/api-contract/tasks'
import { ShellCommand } from '@moatless/shell-utils'
import type { DeployResponse, PublishSettings, Task, TaskListResponse } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { SettingsService } from '../../../services/settings.service'
import { handleServiceError } from '../../../utils/error-handler'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import { AgentController } from '../../agents/services/agent-controller.service'
import { McpService } from '../../agents/services/mcp.service'
import { MessageService } from '../../agents/services/message.service'
import { NetlifyService } from '../../deployments/services/netlify.service'
import { RepositoryService } from '../../repositories/services/repository.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import type {
  DockerExecutionTarget,
  ExecutionTarget,
  KubernetesExecutionTarget,
} from '../../workspace/types/workspace-runtime.types'
import { TaskService } from '../services/task.service'

type DeploymentInfo = Pick<DeployResponse, 'success' | 'siteId' | 'siteUrl'>

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

  // Send message to task
  app.openapi(sendMessageRoute, async (c) => {
    const taskService = appContainer.get(TaskService)

    const { taskId } = c.req.valid('param')
    const { message, resume, model, fileIds, contexts, commandName, agentMode, background } =
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
      background,
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
    const agentController = appContainer.get(AgentController)
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const { taskId } = c.req.valid('param')

    try {
      // Check if agent is currently running
      const snapshot = await runtimeManager.getSnapshot(taskId)
      const wasRunning = snapshot?.agent?.state === 'running'

      // Stop agent (no-op if not running)
      await agentController.stopAgent(taskId)

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

  // MCP (Model Context Protocol) endpoint for task-scoped tools
  app.openapi(mcpRoute, async (c) => {
    const mcpService = appContainer.get(McpService)
    const { taskId } = c.req.valid('param')

    try {
      // Parse JSON-RPC request body
      const requestBody = await c.req.json()

      // Handle the request through MCP service with task owner context
      const response = await mcpService.handleRequestWithTaskContext(taskId, requestBody)

      return c.json(response as unknown as Record<string, unknown>)
    } catch (error) {
      logger.error(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'MCP request error',
      )
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
    logger.info(
      {
        taskId,
        siteId: result.siteId,
        siteUrl: result.siteUrl,
        iterationNumber:
          updatedTask.iterations.length > 0
            ? updatedTask.iterations[updatedTask.iterations.length - 1]?.iterationNumber
            : undefined,
      },
      'Updated task with deployment info',
    )
  }

  function logDeploymentUpdateError(taskId: string, error: unknown): void {
    logger.warn(
      {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      },
      'Failed to update task with deployment info',
    )
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
    logger.error(
      {
        err: error as Error,
        taskId,
        taskName: task?.name,
        error: errorMessage,
      },
      'Deployment failed',
    )

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

  /**
   * Get and validate workspace for deployment
   */
  async function getDeploymentWorkspace(
    taskId: string,
    runtimeManager: WorkspaceRuntimeManager,
    _repositoryService: RepositoryService,
  ): Promise<{
    workspacePath: string
    buildCommand: string
    envVars: Record<string, string>
    outputDirectory: string
  }> {
    const snapshot = await runtimeManager.getSnapshot(taskId)
    if (!snapshot) {
      throw new HTTPException(404, { message: 'Workspace not found for task' })
    }

    // Deployment requires local file access for building and uploading
    // This is not supported for remote workspaces (Docker/K8s)
    throw new HTTPException(400, {
      message: 'Deployment is not supported for remote workspaces. Use CI/CD pipelines instead.',
    })
  }

  /**
   * Build project for deployment
   */
  async function buildProjectForDeployment(
    workspacePath: string,
    buildCommand: string,
    envVars: Record<string, string>,
    outputDirectory: string,
    taskId: string,
  ): Promise<{ success: boolean; outputPath: string; logs: string; duration: number }> {
    const workspaceLogger = logger.child({ taskId })
    workspaceLogger.info('Building project for deployment')
    const startTime = Date.now()

    const shellCommand = new ShellCommand({
      workingDirectory: workspacePath,
      env: envVars,
      timeout: 300000, // 5 minute timeout
    })
    const buildResult = await shellCommand.executeWithResult(buildCommand)

    const outputPath = path.join(workspacePath, outputDirectory)
    const outputExists = await pathExists(outputPath)
    const duration = Date.now() - startTime

    workspaceLogger.info(
      {
        success: buildResult.exitCode === 0 && outputExists,
        exitCode: buildResult.exitCode,
        outputExists,
        duration,
        outputPath,
      },
      'Build completed',
    )

    return {
      success: buildResult.exitCode === 0 && outputExists,
      outputPath,
      logs: buildResult.output,
      duration,
    }
  }

  // Deploy task to Netlify
  app.openapi(deployRoute, async (c) => {
    const taskService = appContainer.get(TaskService)
    const settingsService = appContainer.get(SettingsService)
    const runtimeManager = appContainer.get(WorkspaceRuntimeManager)
    const repositoryService = appContainer.get(RepositoryService)
    const netlifyService = appContainer.get(NetlifyService)
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

      // Get and validate workspace for deployment
      const workspace = await getDeploymentWorkspace(taskId, runtimeManager, repositoryService)

      const workspaceLogger = logger.child({ taskId })
      workspaceLogger.info(
        {
          taskName: task.name,
          hasAuthToken: Boolean(netlifyAccessToken),
          buildCommand: workspace.buildCommand,
        },
        'Starting build and deploy to Netlify',
      )

      // Build the project
      const buildResult = await buildProjectForDeployment(
        workspace.workspacePath,
        workspace.buildCommand,
        workspace.envVars,
        workspace.outputDirectory,
        taskId,
      )

      if (!buildResult.success) {
        return c.json({
          success: false,
          error: 'Build failed',
          logs: buildResult.logs,
          buildDuration: buildResult.duration,
        })
      }

      const outputPath = buildResult.outputPath
      const buildDuration = buildResult.duration

      // Deploy to Netlify
      const siteName = task.name || `moatless-${taskId.substring(0, 8)}`
      workspaceLogger.info(
        {
          siteName,
          existingSiteId: task.siteId,
        },
        'Deploying to Netlify',
      )

      const deployResult = await netlifyService.deployToNetlify(
        outputPath,
        siteName,
        netlifyAccessToken,
        task.siteId || undefined,
      )

      workspaceLogger.info(
        {
          success: deployResult.success,
          deployUrl: deployResult.deployUrl,
          siteUrl: deployResult.siteUrl,
        },
        'Deployment completed',
      )

      // Combine build and deploy results
      const result = {
        ...deployResult,
        buildDuration,
        logs: `Build:\n${buildResult.logs}\n\nDeploy:\n${deployResult.logs}`,
      }

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

  // Note: Screenshot routes removed - screenshots are now accessed via /api/v1/uploads/{fileId}
  // using the screenshotFileId stored in task iterations
}
