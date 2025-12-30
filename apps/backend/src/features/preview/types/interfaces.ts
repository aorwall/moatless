/**
 * Dependency injection interfaces for preview server
 * Note: Logger is now provided by pino (import from utils/logger.ts)
 */

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
  targetUrl?: string | undefined // Full URL for remote targets (Kubernetes worker pods)
  scriptContent?: string | undefined // Optional inspector script to inject
}

// Re-export inspector types from @moatless/types for compatibility
export type { SourceLocation } from '@moatless/types'
