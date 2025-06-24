import { z } from 'zod'

// UUID validation
export const uuidSchema = z.string().uuid()

// Date validation and parsing
export const dateSchema = z
  .union([z.string().datetime(), z.date()])
  .transform((val) => (val instanceof Date ? val : new Date(val)))

// Common validation functions
export function isValidUuid(value: string): boolean {
  return uuidSchema.safeParse(value).success
}

export function parseDate(value: string | Date): Date {
  return dateSchema.parse(value)
}

// Type utilities
export type DeepPartial<T> = {
  [P in keyof T]?: T[P] extends object ? DeepPartial<T[P]> : T[P]
}

export type RequireAtLeastOne<T> = {
  [K in keyof T]-?: Required<Pick<T, K>> & Partial<Pick<T, Exclude<keyof T, K>>>
}[keyof T]

// JSON serialization helpers
export function toJSON<T>(data: T): string {
  return JSON.stringify(data, (_key, value) => {
    if (value instanceof Date) {
      return value.toISOString()
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
