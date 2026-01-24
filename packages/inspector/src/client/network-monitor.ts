import { sendMessage } from './communication'

/** Install fetch interceptor to monitor network requests */
export function installNetworkMonitor(): void {
  const originalFetch = window.fetch

  window.fetch = async (...args: Parameters<typeof fetch>): Promise<Response> => {
    const startTime = performance.now()

    try {
      const response = await originalFetch(...args)

      sendMessage({
        type: 'NETWORK_REQUEST',
        payload: {
          url: (typeof args[0] === 'string' ? args[0] : args[0]?.url) ?? response.url,
          method: (args[1] as RequestInit | undefined)?.method ?? 'GET',
          status: response.status,
          statusText: response.statusText,
          duration: performance.now() - startTime,
          timestamp: new Date().toISOString(),
          origin: location.origin,
        },
      })

      return response
    } catch (error) {
      const err = error as Error

      sendMessage({
        type: 'NETWORK_REQUEST',
        payload: {
          url: typeof args[0] === 'string' ? args[0] : (args[0] as Request).url,
          method: (args[1] as RequestInit | undefined)?.method ?? 'GET',
          error: { message: err.message, stack: err.stack },
          duration: performance.now() - startTime,
          timestamp: new Date().toISOString(),
          origin: location.origin,
        },
      })

      throw error
    }
  }
}
