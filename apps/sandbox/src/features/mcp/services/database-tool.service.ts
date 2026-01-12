import { logger } from '../../../shared/logger'
import type { McpToolResult } from '../types/mcp.types'
import type { BackendApiClient } from './backend-api-client'

/**
 * Database tool service
 * Handles tools that require backend API calls for database operations
 */
export class DatabaseToolService {
  private readonly logger = logger.child({ service: 'database-tool' })

  constructor(private readonly backendClient: BackendApiClient) {}

  /**
   * Handle updateTaskMetadata tool
   * Proxies to backend API with task-scoped authentication
   */
  async handleUpdateTaskMetadata(args: {
    name?: string
    description?: string
  }): Promise<McpToolResult> {
    this.logger.debug({ updates: args }, 'Handling updateTaskMetadata tool')

    try {
      // Validate inputs
      if (!(args.name || args.description)) {
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

      // Proxy to backend API
      await this.backendClient.updateTaskMetadata(args)

      // Build success message
      const updated: string[] = []
      if (args.name) {
        updated.push('name')
      }
      if (args.description) {
        updated.push('description')
      }

      return {
        content: [
          {
            type: 'text',
            text: `Task metadata updated successfully. Updated fields: ${updated.join(', ')}`,
          },
        ],
        isError: false,
      }
    } catch (error) {
      this.logger.error(
        {
          err: error as Error,
          updates: args,
        },
        'Failed to update task metadata',
      )

      return {
        content: [
          {
            type: 'text',
            text: `Error updating task metadata: ${error instanceof Error ? error.message : String(error)}`,
          },
        ],
        isError: true,
      }
    }
  }
}

/**
 * Factory function to create DatabaseToolService instance
 */
export function createDatabaseToolService(backendClient: BackendApiClient): DatabaseToolService {
  return new DatabaseToolService(backendClient)
}
