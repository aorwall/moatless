import { useSyncExternalStore } from 'react'

/**
 * Returns `true` when an error indicates the backend server is unreachable:
 * an HTTP 503 (Traefik "no available server" while the backend is restarting
 * or unschedulable) or a network-level failure (fetch rejected with a
 * `TypeError` / "Failed to fetch" / status 0).
 *
 * Deliberately scoped to 503 + unreachable. Other server errors (500/502/504)
 * stay ordinary "server error" states handled by `FullPageError`.
 */
export function isServerUnavailable(error: unknown): boolean {
  if (error == null) {
    return false
  }

  // Network-unreachable failures: `fetch` rejects with a `TypeError`.
  if (error instanceof TypeError) {
    return true
  }

  if (typeof error === 'object') {
    const status = (error as { status?: unknown }).status
    if (status === 503 || status === 0) {
      return true
    }

    const message = (error as { message?: unknown }).message
    if (typeof message === 'string' && message.includes('Failed to fetch')) {
      return true
    }
  }

  return false
}

type Listener = () => void

function createServerStatusStore() {
  let unavailable = false
  const listeners = new Set<Listener>()

  return {
    subscribe(listener: Listener): () => void {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    getSnapshot(): boolean {
      return unavailable
    },
    setUnavailable(next: boolean): void {
      if (unavailable === next) {
        return
      }
      unavailable = next
      for (const listener of listeners) {
        listener()
      }
    },
  }
}

/**
 * Global, React-tree-independent store tracking whether the backend is
 * reachable. Fed by every API error path (query cache, mutations, and the
 * feature-flags bootstrap) and read by the root-level `ServerDownGate`.
 *
 * It is an external store rather than React context because some consumers
 * (the `QueryCache` callbacks in `query-client.ts`) live outside the React
 * tree and cannot use hooks.
 */
export const serverStatusStore = createServerStatusStore()

export function useServerStatus(): { isUnavailable: boolean } {
  const isUnavailable = useSyncExternalStore(
    serverStatusStore.subscribe,
    serverStatusStore.getSnapshot,
    serverStatusStore.getSnapshot,
  )
  return { isUnavailable }
}
