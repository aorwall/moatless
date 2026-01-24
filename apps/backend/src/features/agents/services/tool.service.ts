import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { TaskService } from '../../tasks/services/task.service'

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
  constructor(@inject(TaskService) private taskFacade: TaskService) {}

  /**
   * Handler for updateTaskMetadata tool
   */
  async handleUpdateTaskMetadata(
    taskId: string,
    args: { name?: string | undefined; description?: string | undefined },
  ): Promise<McpToolResult> {
    logger.info(
      {
        taskId,
        args,
      },
      'Tool execution: updateTaskMetadata',
    )

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
      // Use TaskService.updateTask() which handles vibe repo renaming and event emission
      const updates: { name?: string; description?: string } = {}
      if (name !== undefined) {
        updates.name = name
      }
      if (description !== undefined) {
        updates.description = description
      }

      await this.taskFacade.updateTaskWithEvent(taskId, updates)

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
      logger.error({ err: error as Error, taskId }, 'Failed to update task metadata')

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
}
