import { logger } from '../utils/logger.js'
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

      case 'startPreviewServer':
        return await this.executeStartPreviewServer(taskId, request.id, args)

      case 'stopPreviewServer':
        return await this.executeStopPreviewServer(taskId, request.id, args)

      case 'restartPreviewServer':
        return await this.executeRestartPreviewServer(taskId, request.id, args)

      case 'getPreviewServerStatus':
        return await this.executeGetPreviewServerStatus(taskId, request.id, args)

      case 'getPreviewServerLogs':
        return await this.executeGetPreviewServerLogs(taskId, request.id, args)

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
   * Execute startPreviewServer tool
   */
  private async executeStartPreviewServer(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    try {
      const serverStatus = await this.workspaceService.startServer(taskId)
      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Preview server started successfully. Status: ${serverStatus.running ? 'running' : 'stopped'}${
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
   * Execute stopPreviewServer tool
   */
  private async executeStopPreviewServer(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    try {
      const stopped = await this.workspaceService.stopServer(taskId)
      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: stopped
                ? 'Preview server stopped successfully'
                : 'Preview server was not running or failed to stop',
            },
          ],
        },
      }
    } catch (error) {
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to stop preview server',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute restartPreviewServer tool
   */
  private async executeRestartPreviewServer(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    try {
      const serverStatus = await this.workspaceService.restartServer(taskId)
      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Preview server restarted successfully. Status: ${serverStatus.running ? 'running' : 'stopped'}${
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
        'Failed to restart preview server',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Execute getPreviewServerStatus tool
   */
  private async executeGetPreviewServerStatus(
    taskId: string,
    requestId: string | number | null | undefined,
    _args: any,
  ): Promise<McpResponse> {
    try {
      const serverStatus = await this.workspaceService.getServerStatus(taskId)
      return {
        jsonrpc: '2.0',
        id: requestId,
        result: {
          content: [
            {
              type: 'text',
              text: `Preview server status: ${serverStatus.running ? 'running' : 'stopped'}${
                serverStatus.configured ? '' : ' (not configured)'
              }${serverStatus.url ? `, URL: ${serverStatus.url}` : ''}${
                serverStatus.uptimeSeconds ? `, Uptime: ${serverStatus.uptimeSeconds}s` : ''
              }`,
            },
          ],
        },
      }
    } catch (error) {
      return this.createErrorResponse(
        requestId,
        -32603,
        'Failed to get preview server status',
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
    try {
      const { lines = 100 } = args
      const logs = await this.workspaceService.getServerLogs(taskId, lines)
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
   * Check if preview server is available for the task
   */
  private async isPreviewServerAvailable(taskId: string): Promise<boolean> {
    try {
      const task = await this.taskService.getTask(taskId)
      const repository = await this.taskService['repositoryService'].getRepositoryById(
        task.repositoryId,
      )
      return Boolean(repository?.previewConfig?.serverStartCommand)
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
    ]

    // Add preview server tools if available
    const hasPreviewServer = await this.isPreviewServerAvailable(taskId)
    if (hasPreviewServer) {
      tools.push(
        {
          name: 'startPreviewServer',
          description: 'Start the preview server for this task',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'stopPreviewServer',
          description: 'Stop the preview server for this task',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'restartPreviewServer',
          description: 'Restart the preview server for this task',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'getPreviewServerStatus',
          description: 'Get the current status of the preview server',
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
): McpService {
  return new McpService(taskService, workspaceService)
}
