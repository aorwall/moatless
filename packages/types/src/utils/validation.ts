import { extendZodWithOpenApi } from '@hono/zod-openapi'
import { z } from 'zod'

// Extend Zod with OpenAPI methods
extendZodWithOpenApi(z)

// Date validation and parsing
// Using z.coerce.date() to accept both strings and Date objects from storage
// Add OpenAPI metadata to make it compatible with schema generation
export const dateSchema = z.coerce.date().openapi({
  type: 'string',
  format: 'date-time',
  example: '2024-01-01T00:00:00.000Z',
})

// JSON serialization helpers
export function toJSON<T>(data: T): string {
  const seen = new WeakSet()

  return JSON.stringify(data, (_key, value) => {
    if (value instanceof Date) {
      return value.toISOString()
    }

    // Handle circular references
    if (typeof value === 'object' && value !== null) {
      if (seen.has(value)) {
        return '[Circular Reference]'
      }
      seen.add(value)
    }

    return value
  })
}

export function fromJSON<T>(json: string): T {
  return JSON.parse(json, (_key, value) => {
    if (typeof value === 'string' && /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(value)) {
      return new Date(value)
    }
    return value
  })
}

/**
 * Remove null values from optional fields in an object
 * This ensures compatibility with Zod schemas that use .optional()
 * which expects undefined, not null
 */
export function removeNullsFromOptionalFields<T extends Record<string, unknown>>(
  obj: T,
  optionalFields: string[],
): T {
  if (!obj || typeof obj !== 'object') {
    return obj
  }

  const cleaned = { ...obj } as Record<string, unknown>

  for (const field of optionalFields) {
    if (field in cleaned && cleaned[field] === null) {
      Reflect.deleteProperty(cleaned, field)
    }
  }

  // Handle nested objects like iterations
  if ('iterations' in cleaned && Array.isArray(cleaned['iterations'])) {
    cleaned['iterations'] = cleaned['iterations'].map((iteration: unknown) => {
      if (!iteration || typeof iteration !== 'object') {
        return iteration
      }

      const iterationOptionalFields = [
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

      return removeNullsFromOptionalFields(
        iteration as Record<string, unknown>,
        iterationOptionalFields,
      )
    })
  }

  return cleaned as T
}
