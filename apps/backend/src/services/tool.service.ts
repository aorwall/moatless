import { singleton } from 'tsyringe'
import { RepositoryService } from '../features/repositories/index.js'
import { logger } from '../utils/logger.js'
import { ScreenshotService } from './screenshot.service.js'
import { TaskService } from './task.service.js'
import { WorkspaceService } from './workspace.service.js'

// MCP tool result type
interface McpToolResult {
  content: Array<{
    type: 'text'
    text: string
  }>
  isError?: boolean
}

interface SetupAwareEnvironment {
  areSetupCommandsExecuted(): boolean
  runSetupCommands(): Promise<void>
}

function hasSetupLifecycle(environment: unknown): environment is SetupAwareEnvironment {
  return (
    typeof (environment as { areSetupCommandsExecuted?: unknown }).areSetupCommandsExecuted ===
      'function' &&
    typeof (environment as { runSetupCommands?: unknown }).runSetupCommands === 'function'
  )
}

@singleton()
export class ToolService {
  constructor(
    private taskService: TaskService,
    private workspaceService: WorkspaceService,
    private repositoryService: RepositoryService,
    private screenshotService: ScreenshotService,
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

      await this.taskService.updateTaskMetadata(taskId, updates)

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
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

      // Get current repository config
      const task = await this.taskService.getTask(taskId)
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

      // Update workspace config first
      await workspace.refreshConfiguration(repository)

      // Ensure workspace is ready
      await workspace.setup()

      const setupCommands = repository.codeExecution.setupCommands || []

      // Execute setup commands using the environment's method
      const environment = workspace.getEnvironment()
      await environment.runSetupCommands()

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
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

      // Get latest repository configuration
      const task = await this.taskService.getTask(taskId)
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

      // Refresh workspace configuration with latest repository config
      await workspace.refreshConfiguration(repository)

      // Ensure setup commands are run before starting the preview server
      const environment = workspace.getEnvironment()
      if (hasSetupLifecycle(environment) && !environment.areSetupCommandsExecuted()) {
        logger.info(
          'Setup commands not executed yet, running them before starting preview server',
          {
            taskId,
          },
        )
        await environment.runSetupCommands()
      }

      // Check if server is running
      const currentStatus = await workspace.getServerStatus()
      if (currentStatus.status === 'started' || currentStatus.status === 'starting') {
        // Stop it first
        await workspace.stopServer()
        // Small delay to ensure clean stop
        await new Promise((resolve) => setTimeout(resolve, 1000))
      }

      // Start the server
      const serverStatus = await workspace.startServer()

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
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
      const logs = await workspace.getServerLogs(lines)

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
      const task = await this.taskService.getTask(taskId)
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
      const task = await this.taskService.getTask(taskId)
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
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
      const serverStatus = await workspace.getServerStatus()

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
