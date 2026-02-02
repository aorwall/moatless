import path from 'node:path'
import { ShellCommand } from '@moatless/shell-utils'
import type { DeployResponse, PublishSettings, Task } from '@moatless/types'
import { TaskNotFoundError } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { SettingsService } from '../../../services/settings.service.js'
import { pathExists } from '../../../utils/filesystem.js'
import { logger } from '../../../utils/logger.js'
import { TaskService } from '../../tasks/services/task.service.js'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service.js'
import { NetlifyService } from './netlify.service.js'

type DeploymentInfo = Pick<DeployResponse, 'success' | 'siteId' | 'siteUrl'>

interface DeploymentWorkspace {
  workspacePath: string
  buildCommand: string
  envVars: Record<string, string>
  outputDirectory: string
}

interface BuildResult {
  success: boolean
  outputPath: string
  logs: string
  duration: number
}

export class DeploymentConfigurationError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'DeploymentConfigurationError'
  }
}

export class DeploymentWorkspaceError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'DeploymentWorkspaceError'
  }
}

/**
 * Service for deploying task workspaces to hosting providers.
 * Orchestrates the full deployment flow: validation, build, deploy, and task update.
 */
@injectable()
export class DeploymentService {
  constructor(
    @inject(TaskService) private readonly taskService: TaskService,
    @inject(SettingsService) private readonly settingsService: SettingsService,
    @inject(WorkspaceRuntimeManager) private readonly runtimeManager: WorkspaceRuntimeManager,
    @inject(NetlifyService) private readonly netlifyService: NetlifyService,
  ) {}

  /**
   * Deploy a task workspace to Netlify.
   * Orchestrates the full deployment flow:
   * 1. Validate task exists
   * 2. Validate Netlify configuration
   * 3. Get workspace for deployment
   * 4. Build the project
   * 5. Deploy to Netlify
   * 6. Update task with deployment info
   */
  async deployTask(taskId: string): Promise<DeployResponse> {
    let task: Task | null = null

    try {
      // 1. Get and validate task
      task = await this.taskService.getTask(taskId)
      if (!task) {
        throw new TaskNotFoundError(taskId)
      }

      // 2. Validate Netlify configuration
      const publishSettings = this.settingsService.getPublishSettings()
      this.validateNetlifyToken(publishSettings)

      const netlifyAccessToken = publishSettings.netlifyAccessToken
      if (!netlifyAccessToken) {
        throw new DeploymentConfigurationError('Netlify access token validation failed')
      }

      // 3. Get and validate workspace for deployment
      const workspace = await this.getDeploymentWorkspace(taskId)

      const workspaceLogger = logger.child({ taskId })
      workspaceLogger.info(
        {
          taskName: task.name,
          hasAuthToken: Boolean(netlifyAccessToken),
          buildCommand: workspace.buildCommand,
        },
        'Starting build and deploy to Netlify',
      )

      // 4. Build the project
      const buildResult = await this.buildProject(
        workspace.workspacePath,
        workspace.buildCommand,
        workspace.envVars,
        workspace.outputDirectory,
        taskId,
      )

      if (!buildResult.success) {
        return {
          success: false,
          error: 'Build failed',
          logs: buildResult.logs,
          buildDuration: buildResult.duration,
        }
      }

      const outputPath = buildResult.outputPath
      const buildDuration = buildResult.duration

      // 5. Deploy to Netlify
      const siteName = task.name || `moatless-${taskId.substring(0, 8)}`
      workspaceLogger.info(
        {
          siteName,
          existingSiteId: task.siteId,
        },
        'Deploying to Netlify',
      )

      const deployResult = await this.netlifyService.deployToNetlify(
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
      const result: DeployResponse = {
        ...deployResult,
        buildDuration,
        logs: `Build:\n${buildResult.logs}\n\nDeploy:\n${deployResult.logs}`,
      }

      // 6. Update task with deployment info
      const deploymentInfo: DeploymentInfo = { success: result.success }
      if (result.siteId) {
        deploymentInfo.siteId = result.siteId
      }
      if (result.siteUrl) {
        deploymentInfo.siteUrl = result.siteUrl
      }
      await this.updateTaskWithDeploymentInfo(taskId, deploymentInfo)

      return result
    } catch (error) {
      return this.handleDeploymentError(error, taskId, task ?? undefined)
    }
  }

  /**
   * Validate that Netlify access token is configured
   */
  private validateNetlifyToken(publishSettings: Required<PublishSettings>): void {
    if (!publishSettings.netlifyAccessToken) {
      throw new DeploymentConfigurationError(
        'Netlify access token not configured in settings. Please configure it in Settings > Publish.',
      )
    }
  }

  /**
   * Get and validate workspace for deployment.
   * Currently only supports local workspaces.
   */
  private async getDeploymentWorkspace(taskId: string): Promise<DeploymentWorkspace> {
    const record = await this.runtimeManager.getSandboxRecord(taskId)
    if (!record) {
      throw new DeploymentWorkspaceError('Workspace not found for task')
    }

    // Deployment requires local file access for building and uploading
    // This is not supported for remote workspaces (Docker/K8s)
    throw new DeploymentWorkspaceError(
      'Deployment is not supported for remote workspaces. Use CI/CD pipelines instead.',
    )
  }

  /**
   * Build project for deployment
   */
  private async buildProject(
    workspacePath: string,
    buildCommand: string,
    envVars: Record<string, string>,
    outputDirectory: string,
    taskId: string,
  ): Promise<BuildResult> {
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

  /**
   * Update task with deployment information after successful deployment
   */
  private async updateTaskWithDeploymentInfo(
    taskId: string,
    result: DeploymentInfo,
  ): Promise<void> {
    if (!result.success) {
      return
    }

    try {
      const updatedTask = await this.taskService.getTask(taskId)

      this.updateTaskSiteInfo(updatedTask, result)
      this.updateTaskSiteUrl(updatedTask, result)

      await this.taskService.updateTask(taskId, updatedTask)

      this.logDeploymentUpdate(taskId, result, updatedTask)
    } catch (updateError) {
      this.logDeploymentUpdateError(taskId, updateError)
    }
  }

  private updateTaskSiteInfo(task: Task, result: Pick<DeploymentInfo, 'siteId'>): void {
    if (result.siteId && !task.siteId) {
      task.siteId = result.siteId
      task.siteProvider = 'netlify'
    }
  }

  private updateTaskSiteUrl(task: Task, result: Pick<DeploymentInfo, 'siteUrl'>): void {
    if (result.siteUrl) {
      task.siteUrl = result.siteUrl
    }
  }

  private logDeploymentUpdate(
    taskId: string,
    result: Pick<DeploymentInfo, 'siteId' | 'siteUrl'>,
    updatedTask: Task,
  ): void {
    logger.info(
      {
        taskId,
        siteId: result.siteId,
        siteUrl: result.siteUrl,
        numTurns: updatedTask.numTurns,
      },
      'Updated task with deployment info',
    )
  }

  private logDeploymentUpdateError(taskId: string, error: unknown): void {
    logger.warn(
      {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      },
      'Failed to update task with deployment info',
    )
  }

  private handleDeploymentError(
    error: unknown,
    taskId: string,
    task: Task | undefined,
  ): DeployResponse {
    if (error instanceof TaskNotFoundError) {
      throw error
    }
    if (error instanceof DeploymentConfigurationError) {
      throw error
    }
    if (error instanceof DeploymentWorkspaceError) {
      throw error
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
}
