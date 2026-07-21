import { useEffect, useState } from 'react'
import type { ScriptHealthStatus } from '../types'

/**
 * Health of the inspector script injected into the preview iframe, driven by
 * its INSPECTOR_SCRIPT_LOADED / HEARTBEAT messages. The status degrades to
 * `injected-not-reachable` when no heartbeat arrives for 10 seconds.
 */
export function useScriptHealth() {
  const [scriptHealthStatus, setScriptHealthStatus] = useState<ScriptHealthStatus>('not-injected')
  const [lastHeartbeat, setLastHeartbeat] = useState<number | null>(null)

  useEffect(() => {
    if (lastHeartbeat === null) {
      return
    }

    const heartbeatTimeout = setTimeout(() => {
      setScriptHealthStatus((status) =>
        status === 'injected-reachable' ? 'injected-not-reachable' : status,
      )
    }, 10_000)

    return () => clearTimeout(heartbeatTimeout)
  }, [lastHeartbeat])

  /** INSPECTOR_SCRIPT_LOADED: the script announced itself. */
  const markScriptLoaded = () => {
    setScriptHealthStatus('injected-reachable')
    setLastHeartbeat(Date.now())
  }

  /** HEARTBEAT: the script is still alive. */
  const recordHeartbeat = () => {
    setScriptHealthStatus('injected-reachable')
    setLastHeartbeat(Date.now())
  }

  /**
   * Iframe (re)loaded: back to not-injected until the script announces
   * itself again. Keeps the last heartbeat timestamp.
   */
  const markNotInjected = () => {
    setScriptHealthStatus('not-injected')
  }

  /** Full reset (server changed or stopped). */
  const resetScriptHealth = () => {
    setScriptHealthStatus('not-injected')
    setLastHeartbeat(null)
  }

  return {
    scriptHealthStatus,
    markScriptLoaded,
    recordHeartbeat,
    markNotInjected,
    resetScriptHealth,
  }
}
