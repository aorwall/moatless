/**
 * Dependency injection interfaces for preview server
 * Note: Logger is now provided by pino (import from utils/logger.ts)
 */

/** Event service interface for dependency injection */
export interface EventService {
  emitEvent(event: unknown): Promise<void>
}

export type ResolutionFailureReason =
  | 'no_workspace'
  | 'workspace_stopped'
  | 'workspace_initializing'
  | 'preview_not_started'
  | 'preview_starting'
  | 'pod_not_running'
  | 'connection_refused'

export type ResolveResult =
  | { ok: true; target: TargetInfo }
  | { ok: false; reason: ResolutionFailureReason; subdomain: string; autoStart?: () => void }

/** Target resolver for hostname-based routing */
export interface TargetResolver {
  resolve(taskId: string, serverName?: string): Promise<TargetInfo | null>
  resolveWithContext?(taskId: string, serverName?: string): Promise<ResolveResult>
  /** Lazy status check — called only on proxy error to pick the right error page */
  getServerStatus?(taskId: string, serverName?: string): Promise<ResolutionFailureReason | null>
}

export interface TargetInfo {
  port: number // Target server port
  targetUrl?: string | undefined // Full URL for remote targets (Kubernetes worker pods)
  scriptContent?: string | undefined // Optional inspector script to inject
}

// Re-export inspector types from @moatless/types for compatibility
export type { SourceLocation } from '@moatless/types'
