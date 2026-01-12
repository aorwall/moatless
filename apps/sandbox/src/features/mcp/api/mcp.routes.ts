import { existsSync } from 'node:fs'
import path from 'node:path'
import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { createBackendApiClient } from '../services/backend-api-client'
import { createDatabaseToolService } from '../services/database-tool.service'
import { createMcpService } from '../services/mcp.service'
import { createPreviewToolService } from '../services/preview-tool.service'
import type { McpRequest } from '../types/mcp.types'

/**
 * Check if preview server is available in the workspace
 * Returns true if package.json exists (indicates Node.js project)
 */
function checkHasPreviewServer(workspaceDir: string): boolean {
  const packageJsonPath = path.join(workspaceDir, 'package.json')
  return existsSync(packageJsonPath)
}

/**
 * Register MCP routes
 * POST /mcp - JSON-RPC 2.0 endpoint for MCP protocol
 */
export function registerMcpRoutes(app: Hono<AppBindings>) {
  /**
   * MCP JSON-RPC endpoint
   * Handles initialize, tools/list, and tools/call methods
   */
  app.post('/mcp', async (c) => {
    const config = c.get('config')
    const workspaceDir = c.get('workspaceDir')

    try {
      // Parse JSON-RPC request
      const request = (await c.req.json()) as McpRequest

      logger.debug(
        {
          method: request.method,
          id: request.id,
          taskId: config.taskId,
        },
        'MCP request received',
      )

      // Check if preview server is available by looking for package.json
      const hasPreviewServer = checkHasPreviewServer(workspaceDir)

      logger.debug(
        {
          hasPreviewServer,
          workspaceDir,
          taskId: config.taskId,
        },
        'Preview server availability checked',
      )

      // Create tool services
      const backendApiUrl = process.env['BACKEND_API_URL'] || 'http://backend:8000'
      const taskAuthToken = process.env['TASK_AUTH_TOKEN']

      if (!taskAuthToken) {
        logger.error('TASK_AUTH_TOKEN not found in environment')
        return c.json(
          {
            jsonrpc: '2.0',
            id: request.id,
            error: {
              code: -32603,
              message: 'Server configuration error: Missing task authentication token',
            },
          },
          500,
        )
      }

      const backendClient = createBackendApiClient(config.taskId, backendApiUrl, taskAuthToken)
      const databaseTools = createDatabaseToolService(backendClient)

      // Create preview tools (null PreviewServer since we don't have one yet in MCP context)
      // Screenshot will work with explicit URLs when server is started manually
      const previewTools = await createPreviewToolService(null as never)

      // Create MCP service
      const mcpService = createMcpService(previewTools, databaseTools, hasPreviewServer)

      // Handle request
      const response = await mcpService.handleRequest(request)

      logger.debug(
        {
          method: request.method,
          id: request.id,
          taskId: config.taskId,
          hasError: !!response.error,
        },
        'MCP request handled',
      )

      // Return JSON-RPC response
      return c.json(response)
    } catch (error) {
      logger.error(
        {
          err: error as Error,
          taskId: config.taskId,
        },
        'Failed to handle MCP request',
      )

      return c.json(
        {
          jsonrpc: '2.0',
          error: {
            code: -32603,
            message: 'Internal error',
            data: error instanceof Error ? error.message : String(error),
          },
        },
        500,
      )
    }
  })
}
