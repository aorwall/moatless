import type { UserSession } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { SessionContext } from '../../auth'
import { toolInvocationTotal } from '../../metrics'
import { RepositoryService } from '../../repositories/index'
import { TaskService } from '../../tasks/services/task.service'
import { ToolService } from './tool.service'

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

@injectable()
export class McpService {
  private readonly serverInfo = {
    name: 'moatless-tools',
    version: '1.0.0',
  }

  private readonly protocolVersion = '2024-11-05'

  constructor(
    @inject(ToolService) private toolService: ToolService,
    @inject(TaskService) private taskFacade: TaskService,
    @inject(RepositoryService) private repositoryService: RepositoryService,
  ) {}

  /**
   * Handle MCP request with task owner session context.
   * This method is the entry point for HTTP MCP requests and establishes
   * the appropriate session context before delegating to handleRequest().
   *
   * @param taskId - Task ID for context
   * @param request - MCP JSON-RPC request
   * @returns MCP JSON-RPC response
   */
  async handleRequestWithTaskContext(taskId: string, request: McpRequest): Promise<McpResponse> {
    try {
      // Get task owner ID (bypassing access control for this initial fetch)
      // MCP requests come from Claude Code agent and need task owner context
      const ownerId = await this.taskFacade.getTaskOwner(taskId)

      // Create synthetic session for the task owner
      const session = this.createTaskOwnerSession(taskId, ownerId)

      // Run request within task owner's session context
      return await SessionContext.run(session, () => this.handleRequest(taskId, request))
    } catch (error) {
      logger.error(
        { err: error as Error, taskId, method: request.method },
        'Failed to handle MCP request with task context',
      )
      return this.createErrorResponse(
        request.id,
        -32603,
        'Internal error',
        error instanceof Error ? error.message : String(error),
      )
    }
  }

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
      logger.error(
        { err: error as Error, taskId, method: request.method },
        'Failed to handle MCP request',
      )
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
      logger.error({ err: error as Error, taskId }, 'Failed to get available tools')
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
    const toolName = params?.name ?? 'unknown'
    const args = params?.arguments || {}

    try {
      let response: McpResponse

      switch (toolName) {
        case 'updateTaskMetadata':
          response = await this.executeUpdateTaskMetadata(taskId, request.id, args)
          break

        case 'install':
          response = await this.executeInstall(taskId, request.id, args)
          break

        case 'startOrRestartPreviewServer':
          response = await this.executeStartOrRestartPreviewServer(taskId, request.id, args)
          break

        case 'getPreviewServerLogs':
          response = await this.executeGetPreviewServerLogs(taskId, request.id, args)
          break

        case 'updateRepositoryConfig':
          response = await this.executeUpdateRepositoryConfig(taskId, request.id, args)
          break

        case 'getRepositoryConfig':
          response = await this.executeGetRepositoryConfig(taskId, request.id, args)
          break

        default:
          toolInvocationTotal.labels({ tool_name: toolName, status: 'error' }).inc()
          return this.createErrorResponse(request.id, -32601, `Tool not found: ${toolName}`)
      }

      // Track tool invocation metrics - check if result has isError
      const resultObj = response.result as { isError?: boolean } | undefined
      const status = response.error || resultObj?.isError ? 'error' : 'success'
      toolInvocationTotal.labels({ tool_name: toolName, status }).inc()

      return response
    } catch (error) {
      // Track error metrics
      toolInvocationTotal.labels({ tool_name: toolName, status: 'error' }).inc()
      throw error
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
    args: unknown,
  ): Promise<McpResponse> {
    // Parse force parameter
    const force =
      args && typeof args === 'object' && 'force' in args && typeof args.force === 'boolean'
        ? args.force
        : false

    // Delegate to tool service
    const result = await this.toolService.handleStartOrRestartPreviewServer(taskId, { force })

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
   * Check if preview server is available for the task
   */
  private async isPreviewServerAvailable(taskId: string): Promise<boolean> {
    try {
      const task = await this.taskFacade.getTask(taskId)
      const repository = await this.repositoryService.getRepositoryById(task.repositoryId)
      const hasPreviewServer = (repository?.previewConfig?.serverStartCommands?.length ?? 0) > 0

      logger.debug(
        {
          taskId,
          repositoryId: task.repositoryId,
          hasPreviewServer,
          serverStartCommandsCount: repository?.previewConfig?.serverStartCommands?.length ?? 0,
        },
        'MCP: Preview server availability check result',
      )

      return hasPreviewServer
    } catch (error) {
      logger.warn(
        {
          taskId,
          error,
        },
        'Failed to check preview server availability',
      )
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
            properties: {
              force: {
                type: 'boolean',
                description:
                  'Force restart even if server is running with auto-reload enabled (default: false)',
              },
            },
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
          name: 'navigatePreview',
          description:
            'Navigate the preview browser to a relative path within the preview server. The preview iframe will update to show the navigated page.',
          inputSchema: {
            type: 'object',
            properties: {
              path: {
                type: 'string',
                description:
                  'Relative path to navigate to. Must start with "/" (e.g., "/", "/#/tasks", "/about", "/dashboard"). Absolute URLs are not allowed.',
              },
            },
            required: ['path'],
          },
        },
        {
          name: 'clickElement',
          description:
            'Click an element on the preview page using a CSS selector. Returns screenshot after clicking.',
          inputSchema: {
            type: 'object',
            properties: {
              selector: {
                type: 'string',
                description:
                  'CSS selector for the element to click (e.g., "button.submit", "#login-btn", "a[href=\\"/about\\"]")',
              },
            },
            required: ['selector'],
          },
        },
        {
          name: 'evaluateJavaScript',
          description:
            'Execute JavaScript code in the preview page context. Useful for interacting with the page, inspecting state, or triggering actions.',
          inputSchema: {
            type: 'object',
            properties: {
              script: {
                type: 'string',
                description:
                  'JavaScript code to execute. Has access to the page DOM and can return values.',
              },
            },
            required: ['script'],
          },
        },
        {
          name: 'fillPreviewForm',
          description:
            'Fill multiple form fields at once using CSS selectors. Returns screenshot after filling.',
          inputSchema: {
            type: 'object',
            properties: {
              fields: {
                type: 'object',
                additionalProperties: { type: 'string' },
                description:
                  'Object mapping CSS selectors to values. Example: { "#email": "test@example.com", "#password": "secret123" }',
              },
            },
            required: ['fields'],
          },
        },
        {
          name: 'takePreviewSnapshot',
          description:
            'Capture accessibility snapshot of the current page, this is better than screenshot. Returns page structure and state as text. Maintains browser session across calls.',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'takePreviewScreenshot',
          description:
            'Take a screenshot of the current page. Returns PNG image. Use when you need visual appearance. Maintains browser session across calls.',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'typeText',
          description:
            'Type text into an input field on the preview page using a CSS selector. Can optionally clear existing text before typing and add delay between keystrokes for realistic input simulation.',
          inputSchema: {
            type: 'object',
            properties: {
              selector: {
                type: 'string',
                description:
                  'CSS selector for the input field (e.g., "#email", "input[name=\'username\']", "textarea.comment")',
              },
              text: {
                type: 'string',
                description: 'Text to type into the field',
              },
              delay: {
                type: 'number',
                description: 'Delay in milliseconds between keystrokes (default: 0)',
              },
              clear: {
                type: 'boolean',
                description: 'Clear existing text before typing (default: false)',
              },
            },
            required: ['selector', 'text'],
          },
        },
        {
          name: 'hoverElement',
          description:
            'Hover over an element on the preview page using a CSS selector. Useful for triggering hover states, tooltips, or dropdown menus.',
          inputSchema: {
            type: 'object',
            properties: {
              selector: {
                type: 'string',
                description:
                  'CSS selector for the element to hover over (e.g., ".menu-item", "#tooltip-trigger", "button.dropdown")',
              },
            },
            required: ['selector'],
          },
        },
        {
          name: 'getConsoleLogs',
          description:
            'Get all console logs (log, info, warn, error, debug) that have been captured since the browser session started or since the last call to this tool. Logs are cleared after retrieval.',
          inputSchema: {
            type: 'object',
            properties: {},
          },
        },
        {
          name: 'getNetworkRequests',
          description:
            'Get all network requests that have been captured since the browser session started or since the last call to this tool. Optionally filter by URL pattern. Requests are cleared after retrieval.',
          inputSchema: {
            type: 'object',
            properties: {
              urlPattern: {
                type: 'string',
                description:
                  'Optional regex pattern to filter URLs (e.g., "/api/", "googleapis.com", "\\\\.json$")',
              },
            },
          },
        },
        {
          name: 'waitForElement',
          description:
            'Wait for an element to reach a specific state (attached, detached, visible, hidden) on the preview page. Useful for waiting for dynamic content, lazy-loaded elements, or UI state changes.',
          inputSchema: {
            type: 'object',
            properties: {
              selector: {
                type: 'string',
                description: 'CSS selector for the element to wait for',
              },
              state: {
                type: 'string',
                enum: ['attached', 'detached', 'visible', 'hidden'],
                description:
                  'Element state to wait for: "attached" (in DOM), "detached" (not in DOM), "visible" (visible and not hidden), "hidden" (in DOM but not visible). Default: "visible"',
              },
              timeout: {
                type: 'number',
                description: 'Maximum wait time in milliseconds (default: 30000)',
              },
            },
            required: ['selector'],
          },
        },
      )
    }

    const toolNames = tools.map((t) => t.name)
    logger.debug(
      {
        taskId,
        hasPreviewServer,
        toolCount: tools.length,
        toolNames,
        previewServerTools: hasPreviewServer
          ? [
              'startOrRestartPreviewServer',
              'getPreviewServerLogs',
              'navigatePreview',
              'clickElement',
              'evaluateJavaScript',
              'fillPreviewForm',
              'takePreviewSnapshot',
            ]
          : [],
      },
      'MCP: Returning available tools',
    )

    return tools
  }

  /**
   * Create synthetic session for task owner.
   * Used when MCP requests need to run with task owner's permissions.
   *
   * @param taskId - Task ID for session identification
   * @param createdBy - User ID of task owner
   * @returns UserSession or undefined if task has no owner
   */
  private createTaskOwnerSession(
    taskId: string,
    createdBy: string | undefined,
  ): UserSession | undefined {
    if (!createdBy) {
      return undefined
    }

    return {
      id: `mcp-${taskId}`,
      userId: createdBy,
      userLogin: createdBy,
      userEmail: null,
      userName: null,
      avatarUrl: null,
      role: 'user', // MCP sessions use 'user' role by default
      provider: 'internal' as const,
      createdAt: new Date(),
      expiresAt: new Date(Date.now() + 3600000), // 1 hour
    }
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
