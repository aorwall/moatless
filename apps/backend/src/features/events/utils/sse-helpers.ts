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
 * Subscribe to events and write to SSE stream.
 * Includes error counting to abort dead connections after repeated failures.
 */
export function subscribeToEvents(
  eventService: { subscribeAll: (callback: (event: unknown) => void) => () => void },
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): () => void {
  let writeErrorCount = 0
  const MAX_WRITE_ERRORS = 3

  return eventService.subscribeAll((event) => {
    try {
      if (abortController.signal.aborted) {
        return
      }
      writer.write(new TextEncoder().encode(`${JSON.stringify(event)}\n`))
      sseManager.incrementGlobalEventCount()
      writeErrorCount = 0 // Reset on success
    } catch (error) {
      writeErrorCount++
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed, ignoring write error')
        abortController.abort() // Signal connection is dead
      } else {
        logger.warn(
          { err: error, errorCount: writeErrorCount },
          'Failed to send global event over SSE',
        )
        if (writeErrorCount >= MAX_WRITE_ERRORS) {
          logger.warn('Max write errors reached, aborting SSE connection')
          abortController.abort()
        }
      }
    }
  })
}

/**
 * Start ping interval to keep connection alive.
 * Clears interval on any write error to prevent zombie intervals.
 */
export function startPingInterval(
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): NodeJS.Timeout {
  const interval = setInterval(() => {
    try {
      if (abortController.signal.aborted) {
        clearInterval(interval)
        return
      }
      const pingEvent = {
        type: 'ping',
        timestamp: new Date().toISOString(),
      }
      logger.debug({ pingEvent }, 'Sending ping event')
      writer.write(new TextEncoder().encode(`${JSON.stringify(pingEvent)}\n`))
    } catch (error) {
      // Clear interval on any write error to prevent zombie intervals
      clearInterval(interval)
      if (error instanceof TypeError && error.message.includes('WritableStream')) {
        logger.debug('SSE connection closed during ping, stopped pings')
      } else {
        logger.warn({ err: error }, 'Ping write failed, stopped pings')
      }
    }
  }, 30000)

  return interval
}

/** Maximum connection lifetime when no abort signal is available (1 hour) */
const SSE_MAX_LIFETIME_MS = 3600000

/**
 * Setup disconnect handler for SSE connection.
 * Handles null signal with timeout fallback to prevent connections from never being cleaned up.
 */
export function setupDisconnectHandler(
  signal: AbortSignal | undefined | null,
  unsubscribe: () => void,
  connectionId: string,
  pingInterval: NodeJS.Timeout,
  writer: WritableStreamDefaultWriter<Uint8Array>,
  abortController: AbortController,
): void {
  const cleanup = () => {
    unsubscribe()
    sseManager.disconnectGlobal(connectionId)
    clearInterval(pingInterval)
    try {
      writer.close()
    } catch {
      // Ignore close errors - stream may already be closed
    }
    abortController.abort()
  }

  if (signal) {
    signal.addEventListener('abort', cleanup)
  } else {
    // Fallback: max connection lifetime if no signal available
    logger.warn({ connectionId }, 'No abort signal available, using timeout fallback')
    setTimeout(cleanup, SSE_MAX_LIFETIME_MS)
  }
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
