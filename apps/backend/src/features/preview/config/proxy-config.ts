/**
 * Proxy configuration utilities
 * Centralizes environment variable reading and validation for proxy services
 */

export interface ProxyEnvironmentConfig {
  useHostnameRouting: boolean
  proxyProtocol: 'http' | 'https'
  proxyDomain: string | undefined // Required if useHostnameRouting=true
  proxyPort?: number | undefined
  serverHost: string // Host where preview servers run (default: localhost)
}

/**
 * Load proxy configuration from environment variables
 *
 * Hostname routing is automatically enabled when PROXY_DOMAIN is set.
 * This is the recommended mode for Kubernetes deployments where port-based
 * routing doesn't work (dynamic ports are only accessible inside the pod).
 *
 * You can explicitly disable hostname routing by setting USE_HOSTNAME_ROUTING=false
 * even when PROXY_DOMAIN is set (for testing purposes).
 */
export function loadProxyConfig(): ProxyEnvironmentConfig {
  const proxyDomain = process.env['PROXY_DOMAIN']
  const explicitHostnameRouting = process.env['USE_HOSTNAME_ROUTING']

  // Auto-enable hostname routing when PROXY_DOMAIN is set,
  // unless explicitly disabled via USE_HOSTNAME_ROUTING=false
  const useHostnameRouting =
    explicitHostnameRouting === 'false'
      ? false
      : explicitHostnameRouting === 'true' || !!proxyDomain

  return {
    useHostnameRouting,
    proxyProtocol: (process.env['PROXY_PROTOCOL'] || 'http') as 'http' | 'https',
    proxyDomain,
    proxyPort: process.env['PROXY_PORT']
      ? Number.parseInt(process.env['PROXY_PORT'], 10)
      : undefined,
    serverHost: process.env['SERVER_HOST'] || 'localhost',
  }
}
