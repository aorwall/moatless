const RETURN_TO_KEY = 'auth_return_to'

/**
 * Remember the page the user was trying to reach before being bounced to
 * `/login`, so the OAuth callback can send them back there afterwards.
 * No-op for the dashboard and auth pages themselves.
 */
export function saveReturnTo(): void {
  const path = window.location.pathname + window.location.search
  if (path === '/' || path.startsWith('/login') || path.startsWith('/auth/')) {
    return
  }
  sessionStorage.setItem(RETURN_TO_KEY, path)
}

/**
 * Read and clear the saved return path. Only same-origin absolute paths are
 * accepted (protocol-relative `//host` and full URLs are rejected) so the
 * stored value can never redirect off-site.
 */
export function consumeReturnTo(): string | null {
  const value = sessionStorage.getItem(RETURN_TO_KEY)
  if (value === null) {
    return null
  }
  sessionStorage.removeItem(RETURN_TO_KEY)
  if (value.startsWith('/') && !value.startsWith('//')) {
    return value
  }
  return null
}
