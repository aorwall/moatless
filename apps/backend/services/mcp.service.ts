import { logger } from '../utils/logger.js'
import type { RepositoryService } from './repository.service.js'
import type { TaskService } from './task.service.js'
import type { WorkspaceService } from './workspace.service.js'

// MCP JSON-RPC types following the protocol specification
interface McpRequest {
  jsonrpc: '2.0'
  method: string
  params?: any
  id?: string | number | null
}

interface McpResponse {
  jsonrpc: '2.0'
  id?: string | number | null
  result?: any
  error?: {
    code: number
    message: string
    data?: any
  }
}

interface McpTool {
  name: string
  description: string
  inputSchema: {
    type: 'object'
    properties: Record<string, any>
    required?: string[]
  }
}

export class McpService {
  private readonly serverInfo = {
    name: 'moatless-tools',
    version: '1.0.0',
  }

  private readonly protocolVersion = '2024-11-05'

  constructor(
    private taskService: TaskService,
    private workspaceService: WorkspaceService,
    private repositoryService: RepositoryService,
  ) {}

  /**
   * Handle MCP JSON-RPC request following the protocol specification
   */
  async handleRequest(taskId: string, request: McpRequest): Promise<McpResponse> {
    try {
      switch (request.method) {
        case 'initialize':
          return this.handleInitialize(request)

        case 'tools/list':
          return await this.handleToolsList(taskId, request)

        case 'tools/call':
          return await this.handleToolsCall(taskId, request)

        default:
          return this.createErrorResponse(request.id, -32601, `Method not found: ${request.method}`)
      }
    } catch (error) {
      logger.error('Failed to handle MCP request', error as Error, {
        taskId,
        method: request.method,
      })
      return this.createErrorResponse(
        request.id,
        -32603,
        'Internal error',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Handle initialize method
   */
  private handleInitialize(request: McpRequest): McpResponse {
    return {
      jsonrpc: '2.0',
      id: request.id,
      result: {
        protocolVersion: this.protocolVersion,
        capabilities: {
          tools: {},
        },
        serverInfo: this.serverInfo,
      },
    }
  }

  /**
   * Handle tools/list method
   */
  private async handleToolsList(taskId: string, request: McpRequest): Promise<McpResponse> {
    try {
      const tools = await this.getAvailableTools(taskId)
      return {
        jsonrpc: '2.0',
        id: request.id,
        result: {
          tools,
        },
      }
    } catch (error) {
      logger.error('Failed to get available tools', error as Error, { taskId })
      return this.createErrorResponse(
        request.id,
        -32603,
        'Failed to get available tools',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Handle tools/call method
   */
  private async handleToolsCall(taskId: string, request: McpRequest): Promise<McpResponse> {
    const toolName = request.params?.name
    const args = request.params?.arguments || {}

    switch (toolName) {
      case 'updateTaskMetadata':
        return await this.executeUpdateTaskMetadata(taskId, request.id, args)

      case 'install':
        return await this.executeInstall(taskId, request.id, args)

      case 'startOrRestartPreviewServer':
        return await this.executeStartOrRestartPreviewServer(taskId, request.id, args)

      case 'getPreviewServerLogs':
        return await this.executeGetPreviewServerLogs(taskId, request.id, args)

      case 'updateRepositoryConfig':
        return await this.executeUpdateRepositoryConfig(taskId, request.id, args)

      case 'getRepositoryConfig':
        return await this.executeGetRepositoryConfig(taskId, request.id, args)

      default:
        return this.createErrorResponse(request.id, -32601, `Tool not found: ${toolName}`)
    }
  }

  /**
   * Execute updateTaskMetadata tool
   */
  private async executeUpdateTaskMetadata(
    taskId: string,
    requestId: string | number | null | undefined,
    args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: updateTaskMetadata', { taskId, requestId, args })
    const { name, description } = args

    if (!name && !description) {
      return this.createErrorResponse(
        requestId,
        -32602,
        'At least one field (name or description) must be provided',
      )
    }

    // Update task metadata via TaskService
    await this.taskService.updateTaskMetadata(taskId, { name, description })

    const updatedFields = []
    if (name !== undefined) updatedFields.push('name')
    if (description !== undefined) updatedFields.push('description')

    return {
      jsonrpc: '2.0',
      id: requestId,
      result: {
        content: [
          {
            type: 'text',
            text: `Successfully updated task metadata: ${updatedFields.join(', ')}`,
          },
        ],
      },
    }
  }

  /**
   * Execute startOrRestartPreviewServer tool
   */
  private async executeStartOrRestartPreviewServer(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: startOrRestartPreviewServer', {
      taskId,
      requestId,
      args: _args,
    })
    try {
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

      // Get latest repository configuration
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return this.createErrorResponse(requestId, -32603, 'Repository not found for task')
      }

      // Refresh workspace configuration with latest repository config
      await workspace.refreshConfiguration(repository)

      // Ensure setup commands are run before starting the preview server
      const environment = workspace.getEnvironment()
      if (
        'areSetupCommandsExecuted' in environment &&
        typeof environment.areSetupCommandsExecuted === 'function'
      ) {
        const setupExecuted = (environment as any).areSetupCommandsExecuted()
        if (!setupExecuted) {
          logger.info(
            'Setup commands not executed yet, running them before starting preview server',
            { taskId },
          )
          if (
            'runSetupCommands' in environment &&
            typeof environment.runSetupCommands === 'function'
          ) {
            await (environment as any).runSetupCommands()
          }
        }
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
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Preview server started. Status: ${serverStatus.status}${
                serverStatus.url ? `, URL: ${serverStatus.url}` : ''
              }`,
            },
          ],
        },
      }
    } catch (error) {
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to start preview server',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute getPreviewServerLogs tool
   */
  private async executeGetPreviewServerLogs(
    taskId: string,
    requestId: string | number | null | undefined,
    args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: getPreviewServerLogs', { taskId, requestId, args })
    try {
      const { lines = 100 } = args
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
      const logs = await workspace.getServerLogs(lines)
      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: logs || 'No logs available',
            },
          ],
        },
      }
    } catch (error) {
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to get preview server logs',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute updateRepositoryConfig tool
   */
  private async executeUpdateRepositoryConfig(
    taskId: string,
    requestId: string | number | null | undefined,
    args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: updateRepositoryConfig', { taskId, requestId, args })
    const { setupCommands, envVars, serverStartCommands } = args

    if (!setupCommands && !envVars && !serverStartCommands) {
      return this.createErrorResponse(
        requestId,
        -32602,
        'At least one field (setupCommands, envVars, or serverStartCommands) must be provided',
      )
    }

    try {
      // Get task to find repository ID
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return this.createErrorResponse(requestId, -32603, 'Repository not found for task')
      }

      // Build updates object, preserving existing values
      const updates: any = {}

      if (setupCommands || envVars) {
        updates.codeExecution = {
          ...repository.codeExecution,
          ...(setupCommands && { setupCommands }),
          ...(envVars && { envVars }),
        }
      }

      if (serverStartCommands) {
        updates.previewConfig = {
          ...repository.previewConfig,
          serverStartCommands,
        }
      }

      // Update repository
      await this.repositoryService.updateRepository(task.repositoryId, updates)

      const updatedFields = []
      if (setupCommands !== undefined) updatedFields.push('setupCommands')
      if (envVars !== undefined) updatedFields.push('envVars')
      if (serverStartCommands !== undefined) updatedFields.push('serverStartCommands')

      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Successfully updated repository configuration: ${updatedFields.join(', ')}`,
            },
          ],
        },
      }
    } catch (error) {
      logger.error('Failed to update repository config', error as Error, { taskId })
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to update repository configuration',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute getRepositoryConfig tool
   */
  private async executeGetRepositoryConfig(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: getRepositoryConfig', { taskId, requestId, args: _args })
    try {
      // Get task to find repository ID
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return this.createErrorResponse(requestId, -32603, 'Repository not found for task')
      }

      const config = {
        name: repository.name,
        path: repository.path,
        setupCommands: repository.codeExecution.setupCommands || [],
        envVars: repository.codeExecution.envVars || {},
        serverStartCommands: repository.previewConfig?.serverStartCommands || [],
      }

      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Repository Configuration:
- Name: ${config.name}
- Path: ${config.path}
- Setup Commands: ${config.setupCommands.length ? config.setupCommands.join(', ') : 'None'}
- Environment Variables: ${Object.keys(config.envVars).length ? Object.keys(config.envVars).join(', ') : 'None'}
- Server Start Commands: ${config.serverStartCommands.length ? config.serverStartCommands.join(', ') : 'None'}`,
            },
          ],
          config,
        },
      }
    } catch (error) {
      logger.error('Failed to get repository config', error as Error, { taskId })
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to get repository configuration',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute install tool
   */
  private async executeInstall(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    logger.info('MCP tool execution: install', { taskId, requestId, args: _args })
    try {
      const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)

      // Get current repository config
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      if (!repository) {
        return this.createErrorResponse(requestId, -32603, 'Repository not found for task')
      }

      // Update workspace config first
      await workspace.refreshConfiguration(repository)

      // Ensure workspace is ready
      await workspace.setup()

      const setupCommands = repository.codeExecution.setupCommands || []

      // Execute setup commands using the environment's method
      const environment = workspace.getEnvironment()
      if ('runSetupCommands' in environment && typeof environment.runSetupCommands === 'function') {
        await (environment as any).runSetupCommands()
      }

      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text:
                setupCommands.length > 0
                  ? `Successfully installed repository with ${setupCommands.length} setup command(s): ${setupCommands.join(', ')}`
                  : 'No setup commands configured for this repository',
            },
          ],
        },
      }
    } catch (error) {
      // Return error with the exception message (which includes command output)
      return this.createErrorResponse(
        requestId,
        -32603,
        'Installation failed',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Check if preview server is available for the task
   */
  private async isPreviewServerAvailable(taskId: string): Promise<boolean> {
    try {
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      return Boolean(repository?.previewConfig?.serverStartCommands?.length)
    } catch (error) {
      logger.warn('Failed to check preview server availability', { taskId, error })
      return false
    }
  }

  /**
   * Get available tools definition
   */
  private async getAvailableTools(taskId: string): Promise<McpTool[]> {
    const tools: McpTool[] = [
      {
        name: 'updateTaskMetadata',
        description: 'Update task metadata (name and description) for the current task',
        inputSchema: {
          type: 'object',
          properties: {
            name: {
              type: 'string',
              description: 'Task name (URL-friendly, alphanumeric and dashes only)',
            },
            description: {
              type: 'string',
              description: 'Task description (max 2 sentences or 25 words)',
            },
          },
        },
      },
      {
        name: 'install',
        description: 'Install the repository by running setup commands',
        inputSchema: {
          type: 'object',
          properties: {},
        },
      },
      {
        name: 'updateRepositoryConfig',
        description:
          'Update repository configuration (setup commands, environment variables, server start commands)',
        inputSchema: {
          type: 'object',
          properties: {
            setupCommands: {
              type: 'array',
              items: { type: 'string' },
              description:
                'Commands to install/setup the repository (e.g., npm install, pip install -r requirements.txt)',
            },
            envVars: {
              type: 'object',
              additionalProperties: { type: 'string' },
              description: 'Environment variables for code execution',
            },
            serverStartCommands: {
              type: 'array',
              items: { type: 'string' },
              description:
                'Commands to start the preview server (e.g., npm run dev, python app.py)',
            },
          },
        },
      },
      {
        name: 'getRepositoryConfig',
        description:
          'Get current repository configuration including setup commands, environment variables, and server start commands',
        inputSchema: {
          type: 'object',
          properties: {},
        },
      },
    ]

    // Add preview server tools if available
    const hasPreviewServer = await this.isPreviewServerAvailable(taskId)
    if (hasPreviewServer) {
      tools.push(
        {
          name: 'startOrRestartPreviewServer',
          description: 'Start or restart the preview server for this task',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'getPreviewServerLogs',
          description: 'Get logs from the preview server',
          inputSchema: {
            type: 'object',
            properties: {
              lines: {
                type: 'number',
                description: 'Number of log lines to retrieve (default: 100)',
              },
            },
          },
        },
      )
    }

    return tools
  }

  /**
   * Create standardized error response
   */
  private createErrorResponse(
    id: string | number | null | undefined,
    code: number,
    message: string,
    data?: any,
  ): McpResponse {
    return {
      jsonrpc: '2.0',
      id,
      error: {
        code,
        message,
        data,
      },
    }
  }
}

export function createMcpService(
  taskService: TaskService,
  workspaceService: WorkspaceService,
  repositoryService: RepositoryService,
): McpService {
  return new McpService(taskService, workspaceService, repositoryService)
}
