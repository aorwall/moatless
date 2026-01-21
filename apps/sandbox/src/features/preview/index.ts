// Routes
export { registerPreviewRoutes } from './api/preview.routes'
// Config
export { loadProxyConfig, type ProxyEnvironmentConfig } from './config/proxy-config'
export { createModifiedHtmlHeaders, injectScriptIntoHtml } from './inspector/html-injector'
// Inspector
export { getPreviewInspectorScript } from './inspector/inspector-script'
// Runtime
export { PreviewServer } from './runtime/preview-server'
// Services
export { createPreviewProxy, PreviewProxyService } from './services/preview-proxy.service'
export {
  getRegistry,
  getRegistryOrUndefined,
  initializeRegistry,
  PreviewServerRegistry,
} from './services/preview-server-registry'
// Types
export type {
  CodeChangeInfo,
  HostnameProxyConfig,
  PlaywrightActionInfo,
  PreviewServerConfig,
  PreviewServerLifecycleSink,
  PreviewServerRecord,
  PreviewServerSnapshot,
  PreviewServerStatus,
  PreviewServerUpdate,
  ProxyConfig,
  ServerStatusChangeInfo,
  ServerStatusType,
} from './types/index'

// Utils
export {
  extractSubdomain,
  handleHtmlResponse,
  handleHtmlResponseStream,
  isAssetRequest,
  isFrameworkInternalRoute,
  isHtmlResponse,
} from './utils/proxy-core'
