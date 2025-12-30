import type { Event, RepositoryConfig, TaskResponse } from '@moatless/types'
import { RepositoryConfigSchema, TaskResponseSchema } from '@moatless/types'
import type { z } from 'zod'
import { EventDatabaseWriter } from './event-database-writer'
import { logger } from './logger'

/**
 * Retry configuration for event emission
 */
const RETRY_CONFIG = {
  maxRetries: 5,
  baseDelayMs: 1000, // 1s, 2s, 4s, 8s, 16s
  maxDelayMs: 30000, // Cap at 30s
  jitterFactor: 0.2, // ±20% randomization
} as const

/**
 * Buffered event for async processing
 */
interface BufferedEvent {
  event: Event
  taskId: string
  addedAt: number
}

export class WorkerApiClient {
  private readonly authToken: string | undefined

  // Event buffer for async processing
  private eventBuffer: BufferedEvent[] = []
  private isProcessingBuffer = false
  private bufferProcessingPromise: Promise<void> | null = null

  // Database fallback for events when HTTP delivery fails
  private readonly dbWriter: EventDatabaseWriter

  constructor(private readonly apiUrl: string) {
    // Use WORKER_AUTH_TOKEN from environment (JWT token for user authentication)
    this.authToken = process.env['WORKER_AUTH_TOKEN']
    this.dbWriter = new EventDatabaseWriter()
    logger.info(
      {
        apiUrl,
        hasAuthToken: Boolean(this.authToken),
        dbFallbackAvailable: this.dbWriter.isAvailable(),
      },
      'WorkerApiClient initialized',
    )
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

  /**
   * Sleep for a given number of milliseconds
   */
  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * Calculate delay with exponential backoff and jitter
   */
  private calculateRetryDelay(attempt: number): number {
    const baseDelay = Math.min(RETRY_CONFIG.baseDelayMs * 2 ** attempt, RETRY_CONFIG.maxDelayMs)
    // Add jitter: ±jitterFactor
    const jitter = baseDelay * RETRY_CONFIG.jitterFactor * (Math.random() * 2 - 1)
    return Math.max(0, baseDelay + jitter)
  }

  /**
   * Emit an event with retry logic.
   * Events are buffered and processed asynchronously to not block agent execution.
   */
  // biome-ignore lint/suspicious/useAwait: Async for interface consistency, processing is deferred
  async emitEvent(taskId: string, event: Event): Promise<void> {
    // Add to buffer
    this.eventBuffer.push({
      event,
      taskId,
      addedAt: Date.now(),
    })

    logger.debug(
      {
        taskId,
        eventType: event.type,
        bufferSize: this.eventBuffer.length,
      },
      'Event added to buffer',
    )

    // Start processing if not already running
    this.processBufferAsync()
  }

  /**
   * Process the event buffer asynchronously.
   * This runs in the background and doesn't block the caller.
   */
  private processBufferAsync(): void {
    if (this.isProcessingBuffer) {
      return // Already processing
    }

    this.isProcessingBuffer = true
    this.bufferProcessingPromise = this.processBuffer()
      .catch((error) => {
        logger.error({ err: error }, 'Unexpected error in buffer processing')
      })
      .finally(() => {
        this.isProcessingBuffer = false
        this.bufferProcessingPromise = null
      })
  }

  /**
   * Process all events in the buffer with retry logic.
   * If HTTP delivery fails after max retries, falls back to database write.
   */
  private async processBuffer(): Promise<void> {
    while (this.eventBuffer.length > 0) {
      const item = this.eventBuffer.shift()
      if (!item) {
        continue // Race condition guard
      }

      try {
        await this.emitEventWithRetry(item.taskId, item.event)
        // Success - item already removed by shift()
      } catch (error) {
        // Max retries exceeded - try database fallback
        const bufferTime = Date.now() - item.addedAt

        if (this.dbWriter.isAvailable()) {
          logger.warn(
            {
              taskId: item.taskId,
              eventType: item.event.type,
              eventSubType: item.event.subType,
              bufferTime,
            },
            'HTTP delivery failed, attempting database fallback',
          )

          const dbSuccess = await this.dbWriter.writeEvent(item.event)
          if (dbSuccess) {
            logger.info(
              {
                taskId: item.taskId,
                eventType: item.event.type,
              },
              'Event saved to database fallback',
            )
            continue // Successfully saved, continue to next event
          }
        }

        // Both HTTP and database failed (or db unavailable)
        logger.error(
          {
            err: error as Error,
            taskId: item.taskId,
            eventType: item.event.type,
            eventSubType: item.event.subType,
            bufferTime,
            dbFallbackAvailable: this.dbWriter.isAvailable(),
          },
          'Event delivery failed after max retries and database fallback, dropping event',
        )
        // Continue processing remaining events
      }
    }
  }

  /**
   * Emit a single event with retry logic.
   * Throws if all retries are exhausted.
   */
  private async emitEventWithRetry(taskId: string, event: Event): Promise<void> {
    const url = `${this.apiUrl}/api/v1/worker/emit-event`
    let lastError: Error | null = null

    for (let attempt = 0; attempt <= RETRY_CONFIG.maxRetries; attempt++) {
      try {
        if (attempt > 0) {
          logger.debug(
            {
              taskId,
              eventType: event.type,
              attempt: attempt + 1,
              maxRetries: RETRY_CONFIG.maxRetries + 1,
            },
            'Retrying event emission',
          )
        }

        await this.makeRequest(
          url,
          {
            method: 'POST',
            body: JSON.stringify(event),
          },
          'event',
          taskId,
        )

        logger.debug(
          {
            taskId,
            eventType: event.type,
            attempts: attempt + 1,
          },
          'Event emitted successfully',
        )
        return // Success
      } catch (error) {
        lastError = error as Error

        if (attempt < RETRY_CONFIG.maxRetries) {
          const delay = this.calculateRetryDelay(attempt)
          logger.warn(
            {
              taskId,
              eventType: event.type,
              attempt: attempt + 1,
              maxRetries: RETRY_CONFIG.maxRetries + 1,
              nextRetryMs: Math.round(delay),
              error: lastError.message,
            },
            'Event emission failed, will retry',
          )
          await this.sleep(delay)
        }
      }
    }

    // All retries exhausted
    throw lastError
  }

  /**
   * Wait for all buffered events to be processed.
   * Useful for graceful shutdown or testing.
   */
  async flushEventBuffer(): Promise<void> {
    if (this.bufferProcessingPromise) {
      await this.bufferProcessingPromise
    }
    // Process any remaining events that might have been added
    if (this.eventBuffer.length > 0) {
      await this.processBuffer()
    }
  }

  /**
   * Get the current buffer size (for monitoring/testing)
   */
  getEventBufferSize(): number {
    return this.eventBuffer.length
  }

  /**
   * Close the client and release resources.
   * Flushes the event buffer and closes the database connection.
   */
  async close(): Promise<void> {
    await this.flushEventBuffer()
    await this.dbWriter.close()
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
