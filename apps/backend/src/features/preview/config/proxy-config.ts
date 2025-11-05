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
 * @throws {Error} If hostname routing is enabled but PROXY_DOMAIN is not set
 */
export function loadProxyConfig(): ProxyEnvironmentConfig {
  const useHostnameRouting = process.env['USE_HOSTNAME_ROUTING'] === 'true'
  const proxyDomain = process.env['PROXY_DOMAIN']

  // Validate: if hostname routing enabled, domain is required
  if (useHostnameRouting && !proxyDomain) {
    throw new Error('USE_HOSTNAME_ROUTING=true requires PROXY_DOMAIN to be set')
  }

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
