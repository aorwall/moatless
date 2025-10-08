// Core utilities

export { createModifiedHtmlHeaders, injectScriptIntoHtml } from './core/html-injector.js'
export * from './core/inspector-helpers.js'
export { getPreviewInspectorScript } from './core/inspector-script.js'
export { ProxyCore } from './core/proxy-core.js'
export { HostnameProxyService } from './services/hostname-proxy.js'
export { createPreviewProxy, PreviewProxyService } from './services/preview-proxy.js'
// Services
export { PreviewServer } from './services/preview-server.js'

// Types
export type {
  ElementInfo,
  EventService,
  HostnameProxyConfig,
  Logger,
  PreviewServerConfig,
  PreviewServerStatus,
  ProxyConfig,
  ServerStatusType,
  SourceLocation,
  TargetInfo,
  TargetResolver,
} from './types/index.js'
