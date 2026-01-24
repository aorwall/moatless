import { sendMessage } from './communication'

/** Get the current path, accounting for hash-based routing */
function getCurrentPath(): string {
  let path = window.location.pathname + window.location.search
  if (window.location.hash?.startsWith('#/')) {
    path = window.location.hash
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
