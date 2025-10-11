// Core utilities

export { createModifiedHtmlHeaders, injectScriptIntoHtml } from './core/html-injector.js'
export * from './core/inspector-helpers.js'
export { getPreviewInspectorScript } from './core/inspector-script.js'
export {
  extractSubdomain,
  handleHtmlResponse,
  isAssetRequest,
  isFrameworkInternalRoute,
  isHtmlResponse,
} from './core/proxy-core.js'
export { HostnameProxyService } from './services/hostname-proxy.js'
export { createPreviewProxy, PreviewProxyService } from './services/preview-proxy.js'
// Services
export { PreviewServer } from './services/preview-server.js'
export {
  InMemoryPreviewServerRegistryStore,
  PreviewServerRegistry,
} from './services/preview-server-registry.js'
export { RegistryTargetResolver } from './services/registry-target-resolver.js'

// Types
export type {
  ElementInfo,
  EventService,
  HostnameProxyConfig,
  Logger,
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerRecord,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ProxyConfig,
  ServerStatusChangeInfo,
  ServerStatusType,
  SourceLocation,
  TargetInfo,
  TargetResolver,
} from './types/index.js'
