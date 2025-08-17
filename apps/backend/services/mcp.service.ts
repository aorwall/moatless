import { logger } from '../utils/logger.js'
import type { TaskService } from './task.service.js'

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

  constructor(private taskService: TaskService) {}

  /**
   * Handle MCP JSON-RPC request following the protocol specification
   */
  async handleRequest(taskId: string, request: McpRequest): Promise<McpResponse> {
    try {
      switch (request.method) {
        case 'initialize':
          return this.handleInitialize(request)

        case 'tools/list':
          return this.handleToolsList(request)

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
  private handleToolsList(request: McpRequest): McpResponse {
    return {
      jsonrpc: '2.0',
      id: request.id,
      result: {
        tools: this.getAvailableTools(),
      },
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
   * Get available tools definition
   */
  private getAvailableTools(): McpTool[] {
    return [
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

export function createMcpService(taskService: TaskService): McpService {
  return new McpService(taskService)
}
