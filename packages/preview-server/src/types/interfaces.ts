/**
 * Dependency injection interfaces for preview server
 * These allow apps to provide their own implementations
 */

/** Logger interface for dependency injection */
export interface Logger {
  info(message: unknown, meta?: unknown): void
  error(message: unknown, errorOrMeta?: Error | unknown, meta?: unknown): void
  debug(message: unknown, meta?: unknown): void
  warn(message: unknown, meta?: unknown): void
  child(context: Record<string, unknown>): Logger
}

/** Event service interface for dependency injection */
export interface EventService {
  emitEvent(event: unknown): Promise<void>
}

/** Target resolver for hostname-based routing */
export interface TargetResolver {
  resolve(subdomain: string): Promise<TargetInfo | null>
}

export interface TargetInfo {
  port: number // Target server port
  scriptContent?: string | undefined // Optional inspector script to inject
}

// Re-export inspector types from @moatless/types for compatibility
export type { ElementInfo, SourceLocation } from '@moatless/types'
