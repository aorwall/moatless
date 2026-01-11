// Preview Feature Barrel Exports
// This feature owns preview server lifecycle, runtime management, and inspector tooling

// API Routes
export { registerPreviewRoutes } from './api/preview.routes.js'
export { createModifiedHtmlHeaders, injectScriptIntoHtml } from './inspector/html-injector.js'
export * from './inspector/inspector-helpers.js'
// Inspector Utilities (internal, but exported for testing)
export { getPreviewInspectorScript } from './inspector/inspector-script.js'
// Runtime
export { RegistryTargetResolver } from './runtime/registry-target-resolver.js'
// Services
export { HostnameProxyService } from './services/hostname-proxy.service.js'
export { PlaywrightScreenshotService } from './services/playwright-screenshot.service.js'
export { PreviewOrchestratorService } from './services/preview-orchestrator.service.js'
export { PreviewProxyService } from './services/preview-proxy.service.js'
export { PreviewRuntimeService } from './services/preview-runtime.service.js'
// Types and Interfaces
export type {
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerRecord,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ServerStatusChangeInfo,
  ServerStatusType,
} from './types/index.js'

// Proxy Utilities
export {
  extractSubdomain,
  handleHtmlResponse,
  isAssetRequest,
  isFrameworkInternalRoute,
  isHtmlResponse,
} from './utils/proxy-core.js'
