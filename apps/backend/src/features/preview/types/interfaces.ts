/**
 * Dependency injection interfaces for preview server
 * Note: Logger is now provided by pino (import from utils/logger.ts)
 */

/** Event service interface for dependency injection */
export interface EventService {
  emitEvent(event: unknown): Promise<void>
}

// Re-export inspector types from @moatless/types for compatibility
export type { SourceLocation } from '@moatless/types'
