import { z } from 'zod'

// UUID validation
export const uuidSchema = z.string().uuid()

// Date validation and parsing
export const dateSchema = z
  .union([z.string().datetime(), z.date()])
  .transform((val) => (val instanceof Date ? val : new Date(val)))

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
