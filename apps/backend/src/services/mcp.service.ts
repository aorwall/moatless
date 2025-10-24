import { singleton } from 'tsyringe'
import { logger } from '../utils/logger.js'
import type { RepositoryService } from './repository.service.js'
import type { TaskService } from './task.service.js'
import type { ToolService } from './tool.service.js'

// MCP JSON-RPC types following the protocol specification
interface McpRequest {
  jsonrpc: '2.0'
  method: string
  params?: unknown
  id?: string | number | null
}

interface McpResponse {
  jsonrpc: '2.0'
  id?: string | number | null | undefined
  result?: unknown
  error?: {
    code: number
    message: string
    data?: unknown
  }
}

interface McpTool {
  name: string
  description: string
  inputSchema: {
    type: 'object'
    properties: Record<string, unknown>
    required?: string[]
  }
}

@singleton()
export class McpService {
  private readonly serverInfo = {
    name: 'moatless-tools',
    version: '1.0.0',
  }

  private readonly protocolVersion = '2024-11-05'

  constructor(
    private toolService: ToolService,
    private taskService: TaskService,
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
    const params = request.params as { name?: string; arguments?: unknown } | undefined
    const toolName = params?.name
    const args = params?.arguments || {}

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

      case 'takePreviewScreenshot':
        return await this.executeTakePreviewScreenshot(taskId, request.id, args)

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
    args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleUpdateTaskMetadata(
      taskId,
      args as { name?: string; description?: string },
    )

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute startOrRestartPreviewServer tool
   */
  private async executeStartOrRestartPreviewServer(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleStartOrRestartPreviewServer(taskId)

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute getPreviewServerLogs tool
   */
  private async executeGetPreviewServerLogs(
    taskId: string,
    requestId: string | number | null | undefined,
    args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleGetPreviewServerLogs(
      taskId,
      args as { lines?: number },
    )

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute updateRepositoryConfig tool
   */
  private async executeUpdateRepositoryConfig(
    taskId: string,
    requestId: string | number | null | undefined,
    args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleUpdateRepositoryConfig(
      taskId,
      args as {
        setupCommands?: string[]
        envVars?: Record<string, string>
        serverStartCommands?: string[]
      },
    )

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute getRepositoryConfig tool
   */
  private async executeGetRepositoryConfig(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleGetRepositoryConfig(taskId)

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute install tool
   */
  private async executeInstall(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleInstall(taskId)

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Execute takePreviewScreenshot tool
   */
  private async executeTakePreviewScreenshot(
    taskId: string,
    requestId: string | number | null | undefined,
    args: unknown,
  ): Promise<McpResponse> {
    // Delegate to tool service
    const result = await this.toolService.handleTakePreviewScreenshot(
      taskId,
      args as { path?: string },
    )

    return {
      jsonrpc: '2.0',
      id: requestId,
      result,
    }
  }

  /**
   * Check if preview server is available for the task
   */
  private async isPreviewServerAvailable(taskId: string): Promise<boolean> {
    try {
      const task = await this.taskService.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      const hasPreviewServer = (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0

      logger.info('MCP: Preview server availability check result', {
        taskId,
        repositoryId: task.repositoryId,
        hasPreviewServer,
        serverStartCommandsCount: repository?.previewConfig?.serverStartCommands?.length ?? 0,
      })

      return hasPreviewServer
    } catch (error) {
      logger.warn('Failed to check preview server availability', {
        taskId,
        error,
      })
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
        {
          name: 'takePreviewScreenshot',
          description:
            'Take a screenshot of the preview server. Optionally provide a path to navigate to a specific route before taking the screenshot.',
          inputSchema: {
            type: 'object',
            properties: {
              path: {
                type: 'string',
                description:
                  'Optional relative path to navigate to before taking the screenshot (e.g., "/about", "/dashboard"). If not provided, screenshots the root URL.',
              },
            },
          },
        },
      )
    }

    const toolNames = tools.map((t) => t.name)
    logger.info('MCP: Returning available tools', {
      taskId,
      hasPreviewServer,
      toolCount: tools.length,
      toolNames,
      previewServerTools: hasPreviewServer
        ? ['startOrRestartPreviewServer', 'getPreviewServerLogs', 'takePreviewScreenshot']
        : [],
    })

    return tools
  }

  /**
   * Create standardized error response
   */
  private createErrorResponse(
    id: string | number | null | undefined,
    code: number,
    message: string,
    data?: unknown,
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
