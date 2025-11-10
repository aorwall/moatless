import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { extractErrorMessage } from '../../preview/errors'
import { PlaywrightInteractionService } from '../../preview/services/playwright-interaction.service'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { RepositoryService } from '../../repositories/index'
import { TaskService } from '../../tasks/services/task.service'
import { WorkspaceRuntimeManager } from '../../workspace/services/workspace-runtime-manager.service'

interface McpToolResult {
  content: Array<
    | {
        type: 'text'
        text: string
      }
    | {
        type: 'image'
        data: string // base64 encoded image
        mimeType: string
      }
  >
  isError?: boolean
}

@injectable()
export class ToolService {
  constructor(
    @inject(TaskService) private taskFacade: TaskService,
    @inject(WorkspaceRuntimeManager) private runtimeManager: WorkspaceRuntimeManager,
    @inject(PreviewOrchestratorService) private previewOrchestrator: PreviewOrchestratorService,
    @inject(RepositoryService) private repositoryService: RepositoryService,
    @inject(PlaywrightInteractionService) private playwrightService: PlaywrightInteractionService,
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
      // Get task to check if it's a vibe task
      const task = await this.taskFacade.getTask(taskId)

      // If this is a vibe task and name is being updated, rename the repository
      if (name !== undefined && (task.vibe || task.templateId)) {
        logger.info('Renaming repository for vibe task', {
          taskId,
          repositoryId: task.repositoryId,
          newName: name,
        })

        // Rename the repository (updates display name only, not directory)
        await this.repositoryService.renameRepository(task.repositoryId, name)

        logger.info('Repository renamed successfully for vibe task', {
          taskId,
          repositoryId: task.repositoryId,
          newName: name,
        })
      }

      // Update task metadata
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

      // Check if it's a duplicate name error
      if (error && typeof error === 'object' && 'code' in error) {
        const moatlessError = error as { code: string; message: string }
        if (moatlessError.code === 'REPOSITORY_NAME_CONFLICT') {
          return {
            content: [
              {
                type: 'text',
                text: `Failed to update task: ${moatlessError.message}`,
              },
            ],
            isError: true,
          }
        }
      }

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
      const handle = await this.runtimeManager.ensureWorkspace(taskId)

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
      const needsSetup = handle.preview.needsSetup

      if (setupCommands.length === 0) {
        return {
          content: [
            {
              type: 'text',
              text: 'No setup commands configured for this repository',
            },
          ],
        }
      }

      if (needsSetup) {
        return {
          content: [
            {
              type: 'text',
              text: `Setup commands (${setupCommands.length}) will run automatically on first preview start: ${setupCommands.join(', ')}`,
            },
          ],
        }
      }

      return {
        content: [
          {
            type: 'text',
            text: `Setup commands have already been executed. Commands: ${setupCommands.join(', ')}`,
          },
        ],
      }
    } catch (error) {
      logger.error('Installation query failed', error as Error, { taskId })
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
  async handleStartOrRestartPreviewServer(
    taskId: string,
    args: { force?: boolean } = {},
  ): Promise<McpToolResult> {
    const { force = false } = args
    logger.info('Tool execution: startOrRestartPreviewServer', { taskId, force })

    try {
      // Ensure workspace is provisioned
      await this.runtimeManager.ensureWorkspace(taskId)

      // Delegate to orchestrator with smart restart logic
      const result = await this.previewOrchestrator.startOrRestart(taskId, undefined, force)

      // If restart was skipped, return the skip message
      if (result.skipMessage) {
        return {
          content: [
            {
              type: 'text',
              text: result.skipMessage,
            },
          ],
        }
      }

      // Return success with server status
      return {
        content: [
          {
            type: 'text',
            text: `Preview server started. Status: ${result.status.status}${
              result.status.url ? `, URL: ${result.status.url}` : ''
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
        kind: repository.kind,
        ...(repository.kind === 'local' ? { path: repository.path } : {}),
        ...(repository.kind === 'remote' || repository.kind === 'template'
          ? { remoteUrl: repository.remote.url }
          : {}),
        setupCommands: repository.codeExecution.setupCommands || [],
        envVars: repository.codeExecution.envVars || {},
        serverStartCommands: repository.previewConfig?.serverStartCommands || [],
      }

      const locationInfo =
        repository.kind === 'local'
          ? `- Path: ${repository.path}`
          : `- Remote URL: ${repository.remote.url}`

      return {
        content: [
          {
            type: 'text',
            text: `Repository Configuration:
- Name: ${config.name}
- Kind: ${config.kind}
${locationInfo}
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
   * Handler for navigatePreview tool
   */
  async handleNavigatePreview(taskId: string, args: { path: string }): Promise<McpToolResult> {
    logger.info('Tool execution: navigatePreview', { taskId, path: args.path })

    try {
      // Get internal server URL (bypasses authentication) for Playwright
      // The public API only exposes the external URL (proxy with auth)
      // Note: Server status verification is now handled by playwrightService.navigateTo()
      const internalServerUrl = this.previewOrchestrator.getInternalUrl(taskId)
      if (!internalServerUrl) {
        return {
          content: [
            {
              type: 'text',
              text: 'Error: Preview server is running but internal URL not available',
            },
          ],
          isError: true,
        }
      }

      logger.debug('Using internal server URL for Playwright navigation', {
        taskId,
        internalServerUrl,
      })

      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.navigateTo(taskId, args.path, internalServerUrl)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to navigate preview', error as Error, { taskId })
      const errorMessage = extractErrorMessage(error, `Failed to navigate to ${args.path}`)
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for clickElement tool
   */
  async handleClickElement(taskId: string, args: { selector: string }): Promise<McpToolResult> {
    logger.info('Tool execution: clickElement', { taskId, selector: args.selector })

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.clickElement(taskId, args.selector)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to click element', error as Error, { taskId })
      const errorMessage = extractErrorMessage(error, `Failed to click element "${args.selector}"`)
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for evaluateJavaScript tool
   */
  async handleEvaluateJavaScript(taskId: string, args: { script: string }): Promise<McpToolResult> {
    logger.info('Tool execution: evaluateJavaScript', { taskId, scriptLength: args.script.length })

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.evaluateJavaScript(taskId, args.script)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to evaluate JavaScript', error as Error, { taskId })
      const errorMessage = extractErrorMessage(error, 'Failed to evaluate JavaScript')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for fillPreviewForm tool
   */
  async handleFillPreviewForm(
    taskId: string,
    args: { fields: Record<string, string> },
  ): Promise<McpToolResult> {
    logger.info('Tool execution: fillPreviewForm', {
      taskId,
      fieldCount: Object.keys(args.fields).length,
    })

    try {
      // Method now throws PlaywrightError on failure instead of returning error result
      const result = await this.playwrightService.fillForm(taskId, args.fields)

      return {
        content: [
          {
            type: 'text',
            text: result.message,
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to fill form', error as Error, { taskId })
      const errorMessage = extractErrorMessage(error, 'Failed to fill form')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
          },
        ],
        isError: true,
      }
    }
  }

  /**
   * Handler for takePreviewSnapshot tool
   */
  async handleTakePreviewSnapshot(
    taskId: string,
    args: { includeScreenshot?: boolean } = {},
  ): Promise<McpToolResult> {
    const { includeScreenshot = true } = args
    logger.info('Tool execution: takePreviewSnapshot', { taskId, includeScreenshot })

    try {
      // Use auto-restart version which handles dirty context checking and restart logic
      // Note: takeSnapshotWithAutoRestart fetches repository config internally
      const result = await this.playwrightService.takeSnapshotWithAutoRestart(
        taskId,
        includeScreenshot,
      )

      // Return only the screenshot image (this is the dedicated screenshot tool)
      // Note: screenshotBase64 is guaranteed to be present on success (failures throw exceptions)
      return {
        content: [
          {
            type: 'image',
            data: result.screenshotBase64,
            mimeType: 'image/png',
          },
        ],
      }
    } catch (error) {
      logger.error('Failed to take preview snapshot', error as Error, { taskId })
      const errorMessage = extractErrorMessage(error, 'Failed to take snapshot')
      return {
        content: [
          {
            type: 'text',
            text: errorMessage,
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
