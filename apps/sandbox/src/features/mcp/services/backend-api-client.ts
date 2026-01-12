import { logger } from '../../../shared/logger'

/**
 * HTTP client for communicating with backend API
 * Uses task-scoped JWT authentication
 */
export class BackendApiClient {
  private readonly baseUrl: string
  private readonly taskToken: string
  private readonly taskId: string
  private readonly logger = logger.child({ service: 'backend-api-client' })

  constructor(taskId: string, baseUrl: string, taskToken: string) {
    this.taskId = taskId
    this.baseUrl = baseUrl
    this.taskToken = taskToken
  }

  /**
   * Update task metadata (name and description)
   */
  async updateTaskMetadata(args: { name?: string; description?: string }): Promise<void> {
    const url = `${this.baseUrl}/api/v1/tasks/${this.taskId}/metadata`

    this.logger.debug(
      {
        taskId: this.taskId,
        updates: args,
        url,
      },
      'Updating task metadata via backend API',
    )

    try {
      const response = await fetch(url, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Task-Token': this.taskToken,
        },
        body: JSON.stringify(args),
      })

      if (!response.ok) {
        const errorText = await response.text()
        this.logger.error(
          {
            taskId: this.taskId,
            status: response.status,
            statusText: response.statusText,
            error: errorText,
          },
          'Backend API request failed',
        )
        throw new Error(`Backend API error: ${response.status} - ${errorText}`)
      }

      const result = await response.json()
      this.logger.debug(
        {
          taskId: this.taskId,
          success: result.success,
        },
        'Task metadata updated successfully',
      )
    } catch (error) {
      this.logger.error(
        {
          err: error as Error,
          taskId: this.taskId,
          url,
        },
        'Failed to update task metadata',
      )
      throw error
    }
  }
}

/**
 * Factory function to create BackendApiClient instance
 */
export function createBackendApiClient(
  taskId: string,
  baseUrl: string,
  taskToken: string,
): BackendApiClient {
  return new BackendApiClient(taskId, baseUrl, taskToken)
}
