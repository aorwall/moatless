import { logger } from '../../../shared/logger'
import type { McpRequest, McpResponse, McpTool } from '../types/mcp.types'
import type { DatabaseToolService } from './database-tool.service'
import type { PreviewToolService } from './preview-tool.service'

/**
 * Sandbox MCP service
 * Handles JSON-RPC 2.0 requests for Model Context Protocol
 *
 * Simplified version without:
 * - InversifyJS dependency injection
 * - Session context management (sandbox is single-task)
 * - Prometheus metrics
 * - Repository configuration tools (removed per plan)
 * - Install tool (removed per plan)
 *
 * Tool routing:
 * - Preview tools (11): PreviewToolService (direct execution)
 * - Database tools (1): DatabaseToolService (HTTP proxy to backend)
 */
export class McpService {
  private readonly serverInfo = {
    name: 'moatless-sandbox-tools',
    version: '1.0.0',
  }

  private readonly protocolVersion = '2024-11-05'
  private readonly logger = logger.child({ service: 'mcp' })

  constructor(
    private readonly previewTools: PreviewToolService,
    private readonly databaseTools: DatabaseToolService,
    private readonly hasPreviewServer: boolean,
  ) {
    this.logger.info(
      {
        hasPreviewServer: this.hasPreviewServer,
        serverInfo: this.serverInfo,
        protocolVersion: this.protocolVersion,
      },
      'MCP service initialized',
    )
  }

  /**
   * Handle MCP JSON-RPC request
   */
  async handleRequest(request: McpRequest): Promise<McpResponse> {
    this.logger.debug({ method: request.method, id: request.id }, 'Handling MCP request')

    try {
      switch (request.method) {
        case 'initialize':
          return this.handleInitialize(request)

        case 'tools/list':
          return await this.handleToolsList(request)

        case 'tools/call':
          return await this.handleToolsCall(request)

        default:
          return this.createErrorResponse(request.id, -32601, `Method not found: ${request.method}`)
      }
    } catch (error) {
      this.logger.error(
        { err: error as Error, method: request.method },
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
    this.logger.debug({ id: request.id }, 'Handling initialize')

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
  private async handleToolsList(request: McpRequest): Promise<McpResponse> {
    this.logger.debug({ id: request.id }, 'Handling tools/list')

    try {
      const tools = await this.getAvailableTools()

      this.logger.info(
        {
          id: request.id,
          toolCount: tools.length,
          toolNames: tools.map((t) => t.name),
        },
        'Returning available tools',
      )

      return {
        jsonrpc: '2.0',
        id: request.id,
        result: {
          tools,
        },
      }
    } catch (error) {
      this.logger.error({ err: error as Error, id: request.id }, 'Failed to get available tools')

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
  private async handleToolsCall(request: McpRequest): Promise<McpResponse> {
    const params = request.params as { name?: string; arguments?: unknown } | undefined
    const toolName = params?.name ?? 'unknown'
    const args = params?.arguments || {}

    this.logger.debug({ id: request.id, toolName, args }, 'Handling tools/call')

    try {
      let result: unknown

      // Route to appropriate tool service
      switch (toolName) {
        // Database tools (HTTP proxy to backend)
        case 'updateTaskMetadata':
          result = await this.databaseTools.handleUpdateTaskMetadata(
            args as { name?: string; description?: string },
          )
          break

        // Preview tools (direct execution)
        case 'startOrRestartPreviewServer':
          result = await this.previewTools.handleStartOrRestartPreviewServer(
            args as { force?: boolean },
          )
          break

        case 'getPreviewServerLogs':
          result = await this.previewTools.handleGetPreviewServerLogs(args as { lines?: number })
          break

        case 'navigatePreview':
          result = await this.previewTools.handleNavigatePreview(args as { path: string })
          break

        case 'clickElement':
          result = await this.previewTools.handleClickElement(args as { selector: string })
          break

        case 'evaluateJavaScript':
          result = await this.previewTools.handleEvaluateJavaScript(args as { script: string })
          break

        case 'fillPreviewForm':
          result = await this.previewTools.handleFillPreviewForm(
            args as { fields: Record<string, string> },
          )
          break

        case 'takePreviewSnapshot':
          result = await this.previewTools.handleTakePreviewSnapshot()
          break

        case 'takePreviewScreenshot':
          result = await this.previewTools.handleTakePreviewScreenshot(args as { url?: string })
          break

        case 'typeText':
          result = await this.previewTools.handleTypeText(
            args as { selector: string; text: string; delay?: number; clear?: boolean },
          )
          break

        case 'hoverElement':
          result = await this.previewTools.handleHoverElement(args as { selector: string })
          break

        case 'getConsoleLogs':
          result = await this.previewTools.handleGetConsoleLogs()
          break

        case 'getNetworkRequests':
          result = await this.previewTools.handleGetNetworkRequests(args as { urlPattern?: string })
          break

        case 'waitForElement':
          result = await this.previewTools.handleWaitForElement(
            args as {
              selector: string
              state?: 'attached' | 'detached' | 'visible' | 'hidden'
              timeout?: number
            },
          )
          break

        default:
          return this.createErrorResponse(request.id, -32601, `Tool not found: ${toolName}`)
      }

      this.logger.debug({ id: request.id, toolName }, 'Tool executed successfully')

      return {
        jsonrpc: '2.0',
        id: request.id,
        result,
      }
    } catch (error) {
      this.logger.error({ err: error as Error, id: request.id, toolName }, 'Tool execution failed')

      return this.createErrorResponse(
        request.id,
        -32603,
        `Tool execution failed: ${toolName}`,
        error instanceof Error ? error.message : String(error),
      )
    }
  }

  /**
   * Get available tools definition
   */
  private getAvailableTools(): McpTool[] {
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
    if (this.hasPreviewServer) {
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
            'Take a screenshot of a page. Returns PNG image. Use when you need visual appearance. If the preview server is not managed by the system, provide the URL parameter to screenshot any running server.',
          inputSchema: {
            type: 'object',
            properties: {
              url: {
                type: 'string',
                description:
                  'Optional URL to screenshot (e.g., "http://localhost:5173/"). If not provided, uses the managed preview server URL.',
              },
            },
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

/**
 * Factory function to create McpService instance
 */
export function createMcpService(
  previewTools: PreviewToolService,
  databaseTools: DatabaseToolService,
  hasPreviewServer: boolean,
): McpService {
  return new McpService(previewTools, databaseTools, hasPreviewServer)
}
