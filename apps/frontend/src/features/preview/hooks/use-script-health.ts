import { useCallback, useEffect, useRef, useState } from 'react'
import type { ScriptHealthStatus } from '../types'

/**
 * Health of the inspector script injected into the preview iframe, driven by
 * its INSPECTOR_SCRIPT_LOADED / HEARTBEAT messages. The status degrades to
 * `injected-not-reachable` when no heartbeat arrives for 10 seconds.
 */
export function useScriptHealth() {
  const [scriptHealthStatus, setScriptHealthStatus] = useState<ScriptHealthStatus>('not-injected')
  const [lastHeartbeat, setLastHeartbeat] = useState<number | null>(null)
  const heartbeatTimeoutRef = useRef<NodeJS.Timeout | null>(null)

  // Monitor heartbeat and update script health status
  useEffect(() => {
    // Clear any existing timeout
    if (heartbeatTimeoutRef.current) {
      clearTimeout(heartbeatTimeoutRef.current)
    }

    // If we have a recent heartbeat, set status to reachable
    if (lastHeartbeat && Date.now() - lastHeartbeat < 10_000) {
      setScriptHealthStatus('injected-reachable')
    }

    // Set a timeout to mark script as not reachable if no heartbeat in 10 seconds
    heartbeatTimeoutRef.current = setTimeout(() => {
      if (scriptHealthStatus === 'injected-reachable') {
        setScriptHealthStatus('injected-not-reachable')
      }
    }, 10_000)

    return () => {
      if (heartbeatTimeoutRef.current) {
        clearTimeout(heartbeatTimeoutRef.current)
      }
    }
  }, [lastHeartbeat, scriptHealthStatus])

  /** INSPECTOR_SCRIPT_LOADED: the script announced itself. */
  const markScriptLoaded = useCallback(() => {
    setScriptHealthStatus('injected-reachable')
    setLastHeartbeat(Date.now())
  }, [])

  /** HEARTBEAT: the script is still alive. */
  const recordHeartbeat = useCallback(() => {
    setLastHeartbeat(Date.now())
  }, [])

  /**
   * Iframe (re)loaded: back to not-injected until the script announces
   * itself again. Keeps the last heartbeat timestamp.
   */
  const markNotInjected = useCallback(() => {
    setScriptHealthStatus('not-injected')
  }, [])

  /** Full reset (server changed or stopped). */
  const resetScriptHealth = useCallback(() => {
    setScriptHealthStatus('not-injected')
    setLastHeartbeat(null)
  }, [])

  return {
    scriptHealthStatus,
    markScriptLoaded,
    recordHeartbeat,
    markNotInjected,
    resetScriptHealth,
  }
}
