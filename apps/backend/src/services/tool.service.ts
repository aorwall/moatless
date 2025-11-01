import { inject, injectable } from 'inversify'
import { PreviewOrchestratorService } from '../features/preview/services/preview-orchestrator.service'
import { RepositoryService } from '../features/repositories/index'
import { TaskService } from '../features/tasks/services/task.service'
import { LocalEnvironmentDriver } from '../features/workspace/drivers/local-environment-driver'
import { WorkspaceRuntimeManager } from '../features/workspace/services/workspace-runtime-manager.service'
import { logger } from '../utils/logger'
import { ScreenshotService } from './screenshot.service'

// MCP tool result type
interface McpToolResult {
  content: Array<{
    type: 'text'
    text: string
  }>
  isError?: boolean
}

@injectable()
export class ToolService {
  constructor(
    @inject(TaskService) private taskFacade: TaskService,
    @inject(WorkspaceRuntimeManager) private runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewOrchestratorService) private previewOrchestrator: PreviewOrchestratorService,
    @inject(LocalEnvironmentDriver) private driver: LocalEnvironmentDriver,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(ScreenshotService) private screenshotService: ScreenshotService,
  ) {}

  /**
   * Handler for updateTaskMetadata tool
   */
  async handleUpdateTaskMetadata(
    taskId: string,
    args: { name?: string | undefined; description?: string | undefined },
  ): Promise<McpToolResult> {
    logger.info('Tool execution: updateTaskMetadata', {
      taskId,
      args,
    })

    const { name, description } = args

    if (!(name || description)) {
      return {
        content: [
          {
            type: 'text',
            text: 'Error: At least one field (name or description) must be provided',
          },
        ],
        isError: true,
      }
    }

    try {
      const updates: { name?: string; description?: string } = {}
      if (name !== undefined) {
        updates.name = name
      }
      if (description !== undefined) {
        updates.description = description
      }

      await this.taskFacade.updateTaskMetadata(taskId, updates)

      const updatedFields = []
      if (name !== undefined) {
        updatedFields.push('name')
      }
      if (description !== undefined) {
        updatedFields.push('description')
      }

      return {
        content: [
          {
            type: 'text',
            text: `Successfully updated task metadata: ${updatedFields.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to update task metadata', error as Error, { taskId })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to update task metadata: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for install tool
   */
  async handleInstall(taskId: string): Promise<McpToolResult> {
    logger.info('Tool execution: install', { taskId })

    try {
      // Ensure workspace is provisioned
      const handle = await this.runtimeManager.ensureWorkspace(taskId)

      // Get current repository config
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      const setupCommands = repository.codeExecution.setupCommands || []

      // Execute setup commands
      if (setupCommands.length > 0) {
        await this.driver.executeSetupCommands(
          handle.workspacePath,
          setupCommands,
          repository.codeExecution.envVars,
        )
      }

      return {
        content: [
          {
            type: 'text',
            text:
              setupCommands.length > 0
                ? `Successfully installed repository with ${setupCommands.length} setup command(s): ${setupCommands.join(', ')}`
                : 'No setup commands configured for this repository',
          },
        ],
      }
    } catch (error) {
      logger.error('Installation failed', error as Error, { taskId })
      return {
        content: [
          {
            type: 'text',
            text: `Installation failed: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for startOrRestartPreviewServer tool
   */
  async handleStartOrRestartPreviewServer(taskId: string): Promise<McpToolResult> {
    logger.info('Tool execution: startOrRestartPreviewServer', { taskId })

    try {
      // Ensure workspace is provisioned
      await this.runtimeManager.ensureWorkspace(taskId)

      // Get latest repository configuration
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      // Check if server is running
      const currentStatus = await this.previewOrchestrator.getStatus(taskId)
      if (currentStatus.status === 'started' || currentStatus.status === 'starting') {
        // Stop it first
        await this.previewOrchestrator.stop(taskId)
        // Small delay to ensure clean stop
        await new Promise((resolve) => setTimeout(resolve, 1000))
      }

      // Start the server (PreviewOrchestratorService handles setup commands internally)
      const serverStatus = await this.previewOrchestrator.start(taskId)

      return {
        content: [
          {
            type: 'text',
            text: `Preview server started. Status: ${serverStatus.status}${
              serverStatus.url ? `, URL: ${serverStatus.url}` : ''
            }`,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to start preview server', error as Error, { taskId })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to start preview server: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getPreviewServerLogs tool
   */
  async handleGetPreviewServerLogs(
    taskId: string,
    args: { lines?: number },
  ): Promise<McpToolResult> {
    logger.info('Tool execution: getPreviewServerLogs', {
      taskId,
      args,
    })

    try {
      const { lines = 100 } = args
      const logs = await this.previewOrchestrator.getLogs(taskId, lines)

      return {
        content: [
          {
            type: 'text',
            text: logs || 'No logs available',
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to get preview server logs', error as Error, { taskId })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to get preview server logs: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for updateRepositoryConfig tool
   */
  async handleUpdateRepositoryConfig(
    taskId: string,
    args: {
      setupCommands?: string[]
      envVars?: Record<string, string>
      serverStartCommands?: string[]
    },
  ): Promise<McpToolResult> {
    logger.info('Tool execution: updateRepositoryConfig', {
      taskId,
      args,
    })

    const { setupCommands, envVars, serverStartCommands } = args

    if (!(setupCommands || envVars || serverStartCommands)) {
      return {
        content: [
          {
            type: 'text',
            text: 'Error: At least one field (setupCommands, envVars, or serverStartCommands) must be provided',
          },
        ],
        isError: true,
      }
    }

    try {
      // Get task to find repository ID
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      // Build updates object
      const updates = this.buildRepositoryConfigUpdates(
        repository,
        setupCommands,
        envVars,
        serverStartCommands,
      )

      // Update repository
      await this.repositoryService.updateRepository(task.repositoryId, updates)

      // Get updated field names for success message
      const updatedFields = this.getUpdatedFieldNames(setupCommands, envVars, serverStartCommands)

      return {
        content: [
          {
            type: 'text',
            text: `Successfully updated repository configuration: ${updatedFields.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to update repository config', error as Error, {
        taskId,
      })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to update repository configuration: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for getRepositoryConfig tool
   */
  async handleGetRepositoryConfig(taskId: string): Promise<McpToolResult> {
    logger.info('Tool execution: getRepositoryConfig', { taskId })

    try {
      // Get task to find repository ID
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Repository not found for task',
            },
          ],
          isError: true,
        }
      }

      const config = {
        name: repository.name,
        path: repository.path,
        setupCommands: repository.codeExecution.setupCommands || [],
        envVars: repository.codeExecution.envVars || {},
        serverStartCommands: repository.previewConfig?.serverStartCommands || [],
      }

      return {
        content: [
          {
            type: 'text',
            text: `Repository Configuration:
- Name: ${config.name}
- Path: ${config.path}
- Setup Commands: ${config.setupCommands.length > 0 ? config.setupCommands.join(', ') : 'None'}
- Environment Variables: ${Object.keys(config.envVars).length > 0 ? Object.keys(config.envVars).join(', ') : 'None'}
- Server Start Commands: ${config.serverStartCommands.length > 0 ? config.serverStartCommands.join(', ') : 'None'}`,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to get repository config', error as Error, {
        taskId,
      })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to get repository configuration: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for takePreviewScreenshot tool
   */
  async handleTakePreviewScreenshot(
    taskId: string,
    args: { path?: string } = {},
  ): Promise<McpToolResult> {
    const { path } = args
    logger.info('Tool execution: takePreviewScreenshot', { taskId, path })

    try {
      const serverStatus = await this.previewOrchestrator.getStatus(taskId)

      if (serverStatus.status !== 'started' || !serverStatus.url) {
        return {
          content: [
            {
              type: 'text',
              text: `Error: Preview server is not running. Status: ${serverStatus.status}`,
            },
          ],
          isError: true,
        }
      }

      const screenshotPath = await this.screenshotService.captureSimpleScreenshot(
        serverStatus.url,
        path,
      )

      const pathInfo = path ? ` at path: ${path}` : ''
      return {
        content: [
          {
            type: 'text',
            text: `Screenshot captured successfully${pathInfo}. Path: ${screenshotPath}`,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to capture preview screenshot', error as Error, {
        taskId,
        path,
      })
      return {
        content: [
          {
            type: 'text',
            text: `Failed to capture screenshot: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Build repository config updates object
   */
  private buildRepositoryConfigUpdates(
    repository: {
      codeExecution: Record<string, unknown>
      previewConfig?: Record<string, unknown> | undefined
    },
    setupCommands?: string[],
    envVars?: Record<string, string>,
    serverStartCommands?: string[],
  ): Record<string, unknown> {
    const updates: Record<string, unknown> = {}

    if (setupCommands || envVars) {
      updates['codeExecution'] = {
        ...repository.codeExecution,
        ...(setupCommands && { setupCommands }),
        ...(envVars && { envVars }),
      }
    }

    if (serverStartCommands) {
      updates['previewConfig'] = {
        ...repository.previewConfig,
        serverStartCommands,
      }
    }

    return updates
  }

  /**
   * Get list of updated field names
   */
  private getUpdatedFieldNames(
    setupCommands?: string[],
    envVars?: Record<string, string>,
    serverStartCommands?: string[],
  ): string[] {
    const updatedFields = []
    if (setupCommands !== undefined) {
      updatedFields.push('setupCommands')
    }
    if (envVars !== undefined) {
      updatedFields.push('envVars')
    }
    if (serverStartCommands !== undefined) {
      updatedFields.push('serverStartCommands')
    }
    return updatedFields
  }
}
