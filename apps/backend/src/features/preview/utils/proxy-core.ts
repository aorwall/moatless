/**
 * Extract the subdomain from a hostname given the base proxy domain.
 *
 * Example: extractSubdomain('task-abc.proxy.example.com', 'proxy.example.com') => 'task-abc'
 */
export function extractSubdomain(hostname: string, proxyDomain: string): string | null {
  const suffix = `.${proxyDomain}`
  if (!hostname.endsWith(suffix)) {
    return null
  }
  const subdomain = hostname.slice(0, -suffix.length)
  return subdomain || null
}
