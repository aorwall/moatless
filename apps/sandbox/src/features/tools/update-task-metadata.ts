import { z } from 'zod/v4'
import type { SandboxConfig } from '../../config/environment'
import type { BackendApiClient } from '../../shared/backend-api-client'
import { logger } from '../../shared/logger'
import type { SandboxTool } from './types'

const toolLogger = logger.child({ service: 'tool-update-task-metadata' })

/**
 * Create the updateTaskMetadata tool.
 *
 * Calls PATCH /api/v1/tasks/{taskId}/metadata on the backend via BackendApiClient.
 * Falls back to a no-op warning if the client is not configured.
 */
export function createUpdateTaskMetadataTool(
  config: SandboxConfig,
  backendApi: BackendApiClient | undefined,
): SandboxTool {
  return {
    name: 'updateTaskMetadata',
    description:
      'Update the task name and/or description. Use this to set a concise title and summary for the current task.',
    schema: {
      name: z.string().optional().describe('Short task name / title'),
      description: z.string().optional().describe('Task description or summary'),
    },
    execute: async (args) => {
      if (!(args.name || args.description)) {
        return {
          text: 'At least one of name or description must be provided.',
          isError: true,
        }
      }

      toolLogger.info(
        { taskId: config.taskId, name: args.name, hasDescription: !!args.description },
        'updateTaskMetadata called',
      )

      if (!backendApi) {
        toolLogger.warn(
          { taskId: config.taskId },
          'Backend API client not configured, cannot update task metadata',
        )
        return {
          text: 'Backend API not available. Task metadata update skipped.',
          isError: true,
        }
      }

      try {
        const response = await backendApi.patch(`/api/v1/tasks/${config.taskId}/metadata`, {
          name: args.name,
          description: args.description,
        })

        if (!response.ok) {
          const errorText = await response.text()
          toolLogger.warn(
            { taskId: config.taskId, status: response.status, error: errorText },
            'Failed to update task metadata via backend API',
          )
          return {
            text: `Failed to update task metadata: ${response.status} ${response.statusText}`,
            isError: true,
          }
        }

        toolLogger.debug({ taskId: config.taskId }, 'Task metadata updated via backend API')
        return {
          text: 'Task metadata updated successfully.',
        }
      } catch (error) {
        toolLogger.warn({ err: error, taskId: config.taskId }, 'Error calling backend API')
        return {
          text: 'Failed to update task metadata.',
          isError: true,
        }
      }
    },
  }
}
