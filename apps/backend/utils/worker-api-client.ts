import type { Event, RepositoryConfig, TaskResponse } from '@moatless/types'
import { RepositoryConfigSchema, TaskResponseSchema } from '@moatless/types'
import type { z } from 'zod'
import { createLogger } from './logger.js'

const logger = createLogger()

export class WorkerApiClient {
  constructor(
    private readonly apiUrl: string,
    private readonly apiKey?: string,
  ) {
    logger.info('WorkerApiClient initialized', { apiUrl, hasApiKey: !!apiKey })
  }

  private getHeaders(): Record<string, string> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    }
    if (this.apiKey) {
      headers.Authorization = `Bearer ${this.apiKey}`
    }
    return headers
  }

  /**
   * Transform date strings to Date objects recursively and convert null to undefined for optional fields
   */
  private transformDates(obj: any): any {
    if (obj === null || obj === undefined) {
      return obj
    }

    if (obj instanceof Date) {
      return obj
    }

    if (typeof obj === 'string') {
      // Check if it's an ISO date string
      if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(obj)) {
        return new Date(obj)
      }
      return obj
    }

    if (Array.isArray(obj)) {
      return obj.map((item) => this.transformDates(item))
    }

    if (typeof obj === 'object') {
      const transformed: any = {}
      for (const key in obj) {
        if (Object.hasOwn(obj, key)) {
          const value = obj[key]

          // Convert null to undefined for known optional fields
          if (value === null && this.isOptionalField(key)) {
            continue // Don't set the property, leaving it undefined
          }

          // Transform known date fields
          if (
            key === 'createdAt' ||
            key === 'lastMessageAt' ||
            key === 'startTime' ||
            key === 'endTime' ||
            key === 'timestamp'
          ) {
            transformed[key] = value ? new Date(value) : value
          } else {
            transformed[key] = this.transformDates(value)
          }
        }
      }
      return transformed
    }

    return obj
  }

  /**
   * Check if a field is optional in the schema (can be undefined but not null)
   */
  private isOptionalField(fieldName: string): boolean {
    // List of fields that are optional in the TaskIteration schema
    const optionalFields = [
      'result',
      'endTime',
      'contextTokens',
      'initialMessage',
      'commandName',
      'durationMs',
      'gitDiff',
      'commitHash',
      'merged',
      'agentSessionId',
      'model',
      'gitStatus',
      'gitChanges',
      'gitSyncStatus',
      'attachments',
      'error',
      'willNotResume',
    ]
    return optionalFields.includes(fieldName)
  }

  /**
   * Make a request and handle common error cases
   */
  private async makeRequest<T>(
    url: string,
    options: RequestInit,
    resourceType: string,
    resourceId: string,
  ): Promise<T> {
    logger.debug(`Making ${options.method} request`, { url, resourceType, resourceId })

    try {
      const response = await fetch(url, {
        ...options,
        headers: this.getHeaders(),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(`Failed to ${options.method} ${resourceType}`, {
          resourceId,
          status: response.status,
          statusText: response.statusText,
          error: errorText,
        })
        throw new Error(
          `Failed to ${options.method} ${resourceType} ${resourceId}: ${response.status} ${response.statusText}. Error: ${errorText}`,
        )
      }

      // For non-GET requests that might not return data
      if (options.method !== 'GET') {
        logger.debug(`${resourceType} ${options.method} successful`, { resourceId })
        return {} as T
      }

      const data = await response.json()
      logger.debug(`${resourceType} data received`, {
        resourceId,
        hasData: !!data,
        dataKeys: data ? Object.keys(data) : [],
      })

      return data as T
    } catch (error) {
      logger.error(`Error ${options.method} ${resourceType}`, error as Error, { resourceId })
      throw error
    }
  }

  /**
   * Fetch and parse data with a schema
   */
  private async fetchAndParse<T>(
    url: string,
    schema: z.ZodSchema<T>,
    resourceType: string,
    resourceId: string,
  ): Promise<T> {
    const data = await this.makeRequest<any>(url, { method: 'GET' }, resourceType, resourceId)

    // Transform date strings to Date objects
    const transformedData = this.transformDates(data)
    logger.debug(`${resourceType} data transformed`, {
      resourceId,
      sampleDate: transformedData.createdAt instanceof Date,
    })

    try {
      const parsed = schema.parse(transformedData)
      logger.info(`${resourceType} fetched and parsed successfully`, { resourceId })
      return parsed
    } catch (parseError) {
      logger.error(`Failed to parse ${resourceType} response`, parseError as Error, {
        resourceId,
        rawData: JSON.stringify(data, null, 2).substring(0, 500), // Log first 500 chars
      })
      throw parseError
    }
  }

  async emitEvent(taskId: string, event: Event): Promise<void> {
    const url = `${this.apiUrl}/api/v1/worker/emit-event`
    logger.debug('Emitting event', { taskId, eventType: event.type })

    await this.makeRequest(
      url,
      {
        method: 'POST',
        body: JSON.stringify(event), // Send just the event since it contains taskId
      },
      'event',
      taskId,
    )

    logger.debug('Event emitted successfully', { taskId, eventType: event.type })
  }

  async fetchTask(taskId: string): Promise<TaskResponse> {
    const url = `${this.apiUrl}/api/v1/tasks/${taskId}`
    logger.info('Fetching task', { taskId, url })

    const result = await this.fetchAndParse(url, TaskResponseSchema, 'task', taskId)
    logger.info('Task details', {
      taskId,
      status: result.status,
      iterations: result.iterations.length,
    })

    return result
  }

  async fetchRepository(repositoryId: string): Promise<RepositoryConfig> {
    const url = `${this.apiUrl}/api/v1/repositories/${repositoryId}`
    logger.info('Fetching repository', { repositoryId, url })

    const result = await this.fetchAndParse(url, RepositoryConfigSchema, 'repository', repositoryId)
    logger.info('Repository details', {
      repositoryId,
      type: result.codeExecution?.type,
    })

    return result
  }
}
