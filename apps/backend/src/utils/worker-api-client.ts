import type { Event, RepositoryConfig, TaskResponse } from '@moatless/types'
import { RepositoryConfigSchema, TaskResponseSchema } from '@moatless/types'
import type { z } from 'zod'
import { logger } from './logger'

export class WorkerApiClient {
  private readonly authToken: string | undefined

  constructor(private readonly apiUrl: string) {
    // Use WORKER_AUTH_TOKEN from environment (JWT token for user authentication)
    this.authToken = process.env['WORKER_AUTH_TOKEN']
    logger.info({ apiUrl, hasAuthToken: Boolean(this.authToken) }, 'WorkerApiClient initialized')
  }

  private getHeaders(): Record<string, string> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    }
    if (this.authToken) {
      headers['Authorization'] = `Bearer ${this.authToken}`
    }
    return headers
  }

  /**
   * Transform date strings to Date objects recursively and convert null to undefined for optional fields
   */
  private transformDates(obj: unknown): unknown {
    if (this.isPrimitiveOrDate(obj)) {
      return this.transformPrimitiveValue(obj)
    }

    if (Array.isArray(obj)) {
      return obj.map((item) => this.transformDates(item))
    }

    if (typeof obj === 'object') {
      return this.transformObject(obj)
    }

    return obj
  }

  private isPrimitiveOrDate(obj: unknown): boolean {
    return obj === null || obj === undefined || obj instanceof Date || typeof obj === 'string'
  }

  private transformPrimitiveValue(obj: unknown): unknown {
    if (obj === null || obj === undefined) {
      return obj
    }

    if (obj instanceof Date) {
      return obj
    }

    if (typeof obj === 'string') {
      return this.transformStringToDate(obj)
    }

    return obj
  }

  private transformStringToDate(str: string): Date | string {
    // Check if it's an ISO date string
    if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(str)) {
      return new Date(str)
    }
    return str
  }

  private transformObject(obj: unknown): unknown {
    // At this point we know obj is an object from the caller
    const record = obj as Record<string, unknown>
    const transformed: Record<string, unknown> = {}
    for (const key in record) {
      if (Object.hasOwn(record, key)) {
        const value = record[key]

        // Skip null values for optional fields
        if (this.shouldSkipOptionalField(key, value)) {
          continue
        }

        transformed[key] = this.transformObjectProperty(key, value)
      }
    }
    return transformed
  }

  private shouldSkipOptionalField(key: string, value: unknown): boolean {
    return value === null && this.isOptionalField(key)
  }

  private transformObjectProperty(key: string, value: unknown): unknown {
    if (this.isDateField(key)) {
      if (!value) {
        return value
      }
      // Date fields should be strings that can be converted to Date
      if (typeof value === 'string' || typeof value === 'number') {
        return new Date(value)
      }
      return value
    }
    return this.transformDates(value)
  }

  private isDateField(key: string): boolean {
    return (
      key === 'createdAt' ||
      key === 'lastMessageAt' ||
      key === 'startTime' ||
      key === 'endTime' ||
      key === 'timestamp'
    )
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
    logger.debug(
      {
        url,
        resourceType,
        resourceId,
        method: options.method,
      },
      'Making request',
    )

    try {
      const response = await fetch(url, {
        ...options,
        headers: this.getHeaders(),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error(
          {
            resourceId,
            status: response.status,
            statusText: response.statusText,
            error: errorText,
            method: options.method,
            resourceType,
          },
          'Failed to perform request',
        )
        throw new Error(
          `Failed to ${options.method} ${resourceType} ${resourceId}: ${response.status} ${response.statusText}. Error: ${errorText}`,
        )
      }

      // For non-GET requests that might not return data
      if (options.method !== 'GET') {
        logger.debug(
          {
            resourceId,
            method: options.method,
            resourceType,
          },
          'Request successful',
        )
        return {} as T
      }

      const data = await response.json()
      logger.debug(
        {
          resourceId,
          hasData: !!data,
          dataKeys: data ? Object.keys(data) : [],
          resourceType,
        },
        'Data received',
      )

      return data as T
    } catch (error) {
      logger.error(
        {
          err: error as Error,
          resourceId,
          method: options.method,
          resourceType,
        },
        'Request error',
      )
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
    const data = await this.makeRequest<unknown>(url, { method: 'GET' }, resourceType, resourceId)

    // Transform date strings to Date objects
    const transformedData = this.transformDates(data) as Record<string, unknown>
    logger.debug(
      {
        resourceId,
        sampleDate: transformedData['createdAt'] instanceof Date,
        resourceType,
      },
      'Data transformed',
    )

    try {
      const parsed = schema.parse(transformedData)
      logger.info(
        {
          resourceId,
          resourceType,
        },
        'Fetched and parsed successfully',
      )
      return parsed
    } catch (parseError) {
      logger.error(
        {
          err: parseError as Error,
          resourceId,
          rawData: JSON.stringify(data, null, 2).substring(0, 500), // Log first 500 chars
          resourceType,
        },
        'Failed to parse response',
      )
      throw parseError
    }
  }

  async emitEvent(taskId: string, event: Event): Promise<void> {
    const url = `${this.apiUrl}/api/v1/worker/emit-event`
    logger.debug({ taskId, eventType: event.type }, 'Emitting event')

    await this.makeRequest(
      url,
      {
        method: 'POST',
        body: JSON.stringify(event), // Send just the event since it contains taskId
      },
      'event',
      taskId,
    )

    logger.debug(
      {
        taskId,
        eventType: event.type,
      },
      'Event emitted successfully',
    )
  }

  async fetchTask(taskId: string): Promise<TaskResponse> {
    const url = `${this.apiUrl}/api/v1/tasks/${taskId}`
    logger.info({ taskId, url }, 'Fetching task')

    const result = await this.fetchAndParse(url, TaskResponseSchema, 'task', taskId)
    logger.info(
      {
        taskId,
        status: result.status,
        iterations: result.iterations.length,
      },
      'Task details',
    )

    return result
  }

  async fetchRepository(repositoryId: string): Promise<RepositoryConfig> {
    const url = `${this.apiUrl}/api/v1/repositories/${repositoryId}`
    logger.info({ repositoryId, url }, 'Fetching repository')

    const result = await this.fetchAndParse(url, RepositoryConfigSchema, 'repository', repositoryId)
    logger.info(
      {
        repositoryId,
      },
      'Repository details',
    )

    return result
  }
}
