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
} from '@moatless/api-contract/tasks'
import { ShellCommand } from '@moatless/shell-utils'
import type { DeployResponse, PublishSettings, Task, TaskListResponse } from '@moatless/types'
import { TaskNotFoundError, ValidationError } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { NetlifyService } from '../../../services/netlify.service'
import { SettingsService } from '../../../services/settings.service'
import { getTaskId, handleServiceError } from '../../../utils/error-handler'
import { pathExists } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'
import { AgentController } from '../../agents/services/agent-controller.service'
import { McpService } from '../../agents/services/mcp.service'
import { RepositoryService } from '../../repositories/services/repository.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'
import { TaskService } from '../services/task.service'

type DeploymentInfo = Pick<DeployResponse, 'success' | 'siteId' | 'siteUrl'>

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

  /**
   * Get and validate workspace for deployment
   */
  async function getDeploymentWorkspace(
    taskId: string,
    runtimeManager: WorkspaceRuntimeManager,
    repositoryService: RepositoryService,
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

    if (snapshot.executionTarget.mode !== 'local') {
      throw new HTTPException(400, {
        message: 'Deployment only supported for local workspaces',
      })
    }

    const repository = await repositoryService.getRepositoryById(snapshot.repositoryId)
    if (!repository) {
      throw new HTTPException(404, { message: 'Repository configuration not found' })
    }

    const buildCommand = repository.previewConfig?.buildCommand
    if (!buildCommand) {
      throw new HTTPException(400, {
        message: 'No build command configured in repository preview settings',
      })
    }

    return {
      workspacePath: snapshot.executionTarget.workspacePath,
      buildCommand,
      envVars: repository.codeExecution?.envVars || {},
      outputDirectory: repository.previewConfig?.buildOutputDirectory || 'dist',
    }
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

    workspaceLogger.info('Build completed', {
      success: buildResult.exitCode === 0 && outputExists,
      exitCode: buildResult.exitCode,
      outputExists,
      duration,
      outputPath,
    })

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
      workspaceLogger.info('Starting build and deploy to Netlify', {
        taskName: task.name,
        hasAuthToken: Boolean(netlifyAccessToken),
        buildCommand: workspace.buildCommand,
      })

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
      workspaceLogger.info('Deploying to Netlify', {
        siteName,
        existingSiteId: task.siteId,
      })

      const deployResult = await netlifyService.deployToNetlify(
        outputPath,
        siteName,
        netlifyAccessToken,
        task.siteId || undefined,
      )

      workspaceLogger.info('Deployment completed', {
        success: deployResult.success,
        deployUrl: deployResult.deployUrl,
        siteUrl: deployResult.siteUrl,
      })

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

  // Get task iteration thumbnail
  // Note: This route returns binary image data (image/png) and is not migrated to OpenAPI
  // because binary responses don't fit well with the standard OpenAPI JSON response pattern
  app.get('/api/v1/tasks/:taskId/iterations/:iterationNumber/thumbnail', async (c) => {
    const { ScreenshotService } = await import('../../../services/screenshot.service.js')
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
  app.get('/api/v1/tasks/:taskId/iterations/:iterationNumber/screenshot', async (c) => {
    const { ScreenshotService } = await import('../../../services/screenshot.service.js')
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
