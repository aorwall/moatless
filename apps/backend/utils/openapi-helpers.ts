/**
 * OpenAPI utilities to handle hono-openapi type compatibility issues
 */

import { resolver } from 'hono-openapi/zod'
import type { z } from 'zod'

/**
 * Type-safe wrapper for hono-openapi resolver that handles the type compatibility issue
 * This eliminates the need for `as any` throughout the codebase
 *
 * The `resolver` function from hono-openapi has a type mismatch with OpenAPI schema types.
 * This wrapper provides a cleaner abstraction than spreading `as any` throughout the codebase.
 */
export function createSchema<T extends z.ZodType>(schema: T) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return resolver(schema) as any
}
