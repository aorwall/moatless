import type { CreateTaskRequest, RepositoryConfig, TaskResponse } from '@moatless/types'
import { logger } from '../../utils/logger'

/**
 * E2E Test Client for testing against a deployed API.
 * Uses API key authentication to interact with the remote API.
 */
export class E2EClient {
  private readonly headers: Record<string, string>

  constructor(
    private readonly baseUrl: string,
    apiKey: string,
  ) {
    if (!apiKey) {
      throw new Error('E2E_API_KEY is required for E2E testing')
    }

    this.headers = {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${apiKey}`,
    }
  }

  /**
   * Create a new repository.
   */
  async createRepository(data: {
    name: string
    gitCloneUrl: string
    branch?: string
  }): Promise<RepositoryConfig> {
    const response = await fetch(`${this.baseUrl}/api/v1/repositories`, {
      method: 'POST',
      headers: this.headers,
      body: JSON.stringify({
        name: data.name,
        gitCloneUrl: data.gitCloneUrl,
        branch: data.branch,
      }),
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to create repository: ${response.status} ${errorText}`)
    }

    return (await response.json()) as RepositoryConfig
  }

  /**
   * Get a repository by ID.
   */
  async getRepository(repositoryId: string): Promise<RepositoryConfig> {
    const response = await fetch(`${this.baseUrl}/api/v1/repositories/${repositoryId}`, {
      method: 'GET',
      headers: this.headers,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to get repository: ${response.status} ${errorText}`)
    }

    return (await response.json()) as RepositoryConfig
  }

  /**
   * Delete a repository.
   */
  async deleteRepository(repositoryId: string): Promise<void> {
    const response = await fetch(`${this.baseUrl}/api/v1/repositories/${repositoryId}`, {
      method: 'DELETE',
      headers: this.headers,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to delete repository: ${response.status} ${errorText}`)
    }
  }

  /**
   * Create a new task.
   */
  async createTask(data: CreateTaskRequest): Promise<{ taskId: string }> {
    const response = await fetch(`${this.baseUrl}/api/v1/tasks`, {
      method: 'POST',
      headers: this.headers,
      body: JSON.stringify(data),
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to create task: ${response.status} ${errorText}`)
    }

    return (await response.json()) as { taskId: string }
  }

  /**
   * Get a task by ID.
   */
  async getTask(taskId: string): Promise<TaskResponse> {
    const response = await fetch(`${this.baseUrl}/api/v1/tasks/${taskId}`, {
      method: 'GET',
      headers: this.headers,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to get task: ${response.status} ${errorText}`)
    }

    return (await response.json()) as TaskResponse
  }

  /**
   * Delete a task.
   */
  async deleteTask(taskId: string): Promise<void> {
    const response = await fetch(`${this.baseUrl}/api/v1/tasks/${taskId}`, {
      method: 'DELETE',
      headers: this.headers,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to delete task: ${response.status} ${errorText}`)
    }
  }

  /**
   * Wait for a task to complete.
   * Polls the task status until it reaches a terminal state or timeout.
   *
   * Task statuses: 'open' | 'running' | 'error' | 'merged' | 'closed'
   * Iteration statuses: 'created' | 'active' | 'completed' | 'failed'
   */
  async waitForTaskCompletion(
    taskId: string,
    options: {
      timeout?: number
      pollInterval?: number
    } = {},
  ): Promise<TaskResponse> {
    const timeout = options.timeout ?? 180000 // 3 minutes default
    const pollInterval = options.pollInterval ?? 5000 // 5 seconds

    const startTime = Date.now()

    while (Date.now() - startTime < timeout) {
      const task = await this.getTask(taskId)

      logger.debug(
        { taskId, status: task.status, elapsed: Date.now() - startTime },
        'Polling task status',
      )

      // Check if task has reached a terminal state
      // Task terminal states: 'error', 'merged', 'closed'
      if (task.status === 'error' || task.status === 'merged' || task.status === 'closed') {
        return task
      }

      // Also check iteration status for more granular completion detection
      if (task.iterations.length > 0) {
        const lastIteration = task.iterations[task.iterations.length - 1]
        // Iteration terminal states: 'completed', 'failed'
        if (
          lastIteration &&
          (lastIteration.status === 'completed' || lastIteration.status === 'failed')
        ) {
          return task
        }
      }

      // Wait before next poll
      await new Promise((resolve) => setTimeout(resolve, pollInterval))
    }

    throw new Error(`Task ${taskId} did not complete within ${timeout}ms`)
  }

  /**
   * Check API health/connectivity.
   */
  async healthCheck(): Promise<boolean> {
    try {
      const response = await fetch(`${this.baseUrl}/api/v1`, {
        method: 'GET',
        headers: this.headers,
      })
      return response.ok
    } catch {
      return false
    }
  }
}
