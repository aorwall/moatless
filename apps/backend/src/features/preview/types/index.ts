// Configuration types
export type { HostnameProxyConfig, PreviewServerConfig, ProxyConfig } from './config.js'
// DI interfaces (Logger removed - use pino from utils/logger.ts)
export type {
  EventService,
  ResolutionFailureReason,
  ResolveResult,
  SourceLocation,
  TargetInfo,
  TargetResolver,
} from './interfaces.js'
// Registry types
export type {
  CodeChangeInfo,
  PlaywrightActionInfo,
  PreviewServerLifecycleSink,
  PreviewServerRecord,
  PreviewServerSnapshot,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
} from './registry.js'
// Status types
export type { PreviewServerStatus, ServerStatusType } from './status.js'
