import { sendMessage } from './communication'

/**
 * Upper bound for a reported route. Anything longer is pathological (e.g. a
 * self-preview loop where the host nests this app's URL back into its own
 * `?path=`); report the pathname alone rather than propagating a giant string.
 */
const MAX_REPORTED_PATH_LENGTH = 2048

/** Get the current path, accounting for hash-based routing */
export function getCurrentPath(): string {
  let path = window.location.pathname + window.location.search
  if (window.location.hash?.startsWith('#/')) {
    path = window.location.hash
  }
  if (path.length > MAX_REPORTED_PATH_LENGTH) {
    path = window.location.pathname
  }
  return path
}

/** Install route/navigation change tracking */
export function installRouteTracker(): void {
  let lastPath = getCurrentPath()

  // Report initial route
  sendMessage({ type: 'ROUTE_CHANGE', payload: { path: lastPath, timestamp: Date.now() } })

  const checkForRouteChange = () => {
    const currentPath = getCurrentPath()
    if (currentPath !== lastPath) {
      lastPath = currentPath
      sendMessage({ type: 'ROUTE_CHANGE', payload: { path: lastPath, timestamp: Date.now() } })
    }
  }

  // Listen for browser back/forward navigation
  window.addEventListener('popstate', checkForRouteChange)

  // Intercept history.pushState and history.replaceState
  const originalPushState = history.pushState
  const originalReplaceState = history.replaceState

  history.pushState = (...args: Parameters<typeof history.pushState>) => {
    originalPushState.apply(history, args)
    setTimeout(checkForRouteChange, 0)
  }

  history.replaceState = (...args: Parameters<typeof history.replaceState>) => {
    originalReplaceState.apply(history, args)
    setTimeout(checkForRouteChange, 0)
  }

  // Listen for hash changes
  window.addEventListener('hashchange', checkForRouteChange)
}
