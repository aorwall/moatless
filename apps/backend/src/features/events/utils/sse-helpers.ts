import { logger } from '../../../utils/logger'
import { sseManager } from './sse-manager'

/**
 * Build CORS headers for SSE response
 */
export function buildCorsHeaders(origin: string | undefined): Record<string, string> {
  const corsHeaders: Record<string, string> = {}

  if (origin) {
    return buildCorsHeadersWithOrigin(origin, corsHeaders)
  }

  return buildCorsHeadersWithoutOrigin(corsHeaders)
}

function buildCorsHeadersWithOrigin(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  if (process.env['NODE_ENV'] !== 'production') {
    return buildDevelopmentCorsHeaders(origin, corsHeaders)
  }

  return buildProductionCorsHeaders(origin, corsHeaders)
}

function buildDevelopmentCorsHeaders(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  try {
    const url = new URL(origin)
    if (url.hostname === 'localhost' || url.hostname === '127.0.0.1') {
      corsHeaders['Access-Control-Allow-Origin'] = origin
      corsHeaders['Access-Control-Allow-Credentials'] = 'true'
    }
  } catch {
    // Invalid origin URL, skip CORS headers
  }
  return corsHeaders
}

function buildProductionCorsHeaders(
  origin: string,
  corsHeaders: Record<string, string>,
): Record<string, string> {
  const allowedOrigin = process.env['CORS_ORIGIN']
  if (allowedOrigin === 'true' || allowedOrigin === origin) {
    corsHeaders['Access-Control-Allow-Origin'] = origin
    corsHeaders['Access-Control-Allow-Credentials'] = 'true'
  }
  return corsHeaders
}

function buildCorsHeadersWithoutOrigin(
  corsHeaders: Record<string, string>,
): Record<string, string> {
  if (process.env['CORS_ORIGIN'] === 'true' || process.env['NODE_ENV'] !== 'production') {
    corsHeaders['Access-Control-Allow-Origin'] = '*'
  }
  return corsHeaders
}

/**
 * Register an SSE connection with the manager
 */
export function registerSseConnection(
  connectionId: string,
  writer: WritableStreamDefaultWriter<Uint8Array>,
): void {
  const controller = {
    enqueue: (data: string) => writer.write(new TextEncoder().encode(data)),
    close: () => writer.close(),
    error: (error: Error) => writer.abort(error),
    desiredSize: 1,
  } as ReadableStreamDefaultController
  sseManager.connectGlobal(connectionId, controller)
}

/**
 * Subscribe to events and write to SSE stream
 */
export function subscribeToEvents(
  eventService: { subscribeAll: (callback: (event: unknown) => void) => () => void },
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): () => void {
  return eventService.subscribeAll((event) => {
    try {
      if (abortController.signal.aborted) {
        return
      }
      writer.write(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
      sseManager.incrementGlobalEventCount()
    } catch (error) {
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed, ignoring write error')
      } else {
        logger.error('Failed to send global event over SSE', error)
      }
    }
  })
}

/**
 * Start ping interval to keep connection alive
 */
export function startPingInterval(
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): NodeJS.Timeout {
  return setInterval(() => {
    try {
      if (abortController.signal.aborted) {
        return
      }
      const pingEvent = {
        type: 'ping',
        timestamp: new Date().toISOString(),
      }
      logger.debug('Sending ping event', { pingEvent })
      writer.write(new TextEncoder().encode(`${JSON.stringify(pingEvent)}\n`))
    } catch (error) {
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed during ping, stopping pings')
      }
    }
  }, 30000)
}

/**
 * Setup disconnect handler for SSE connection
 */
export function setupDisconnectHandler(
  signal: AbortSignal | undefined | null,
  unsubscribe: () => void,
  connectionId: string,
  pingInterval: NodeJS.Timeout,
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): void {
  signal?.addEventListener('abort', () => {
    unsubscribe()
    sseManager.disconnectGlobal(connectionId)
    clearInterval(pingInterval)
    writer.close()
    abortController.abort()
  })
}

/**
 * Build SSE stream response
 */
export function buildStreamResponse(
  readable: ReadableStream,
  corsHeaders: Record<string, string>,
): Response {
  return new Response(readable, {
    status: 200,
    headers: {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no',
      ...corsHeaders,
    },
  })
}
