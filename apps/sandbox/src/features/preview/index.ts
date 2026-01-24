// Routes
export { registerPreviewRoutes } from './api/preview.routes'
// Config
export { loadProxyConfig, type ProxyEnvironmentConfig } from './config/proxy-config'
export { createModifiedHtmlHeaders, injectScriptIntoHtml } from './inspector/html-injector'
// Inspector
export { getPreviewInspectorScript } from './inspector/inspector-script'
// Runtime
export { handleProxyRequest } from './runtime/preview-proxy'
// Services
export {
  getMonitorOrUndefined,
  initializeMonitor,
  PreviewProcessMonitor,
} from './services/preview-process-monitor'
// Types
export type {
  PreviewServerStatus,
  ServerStatusType,
} from './types/index'

// Utils
export {
  extractSubdomain,
  handleHtmlResponse,
  isAssetRequest,
  isFrameworkInternalRoute,
  isHtmlResponse,
} from './utils/proxy-core'
