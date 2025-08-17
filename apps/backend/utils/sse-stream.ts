import type { Context } from 'hono'

export interface SSEEvent {
  type: string
  data: any
}

/**
 * Creates an SSE response stream for async generators
 * Can be reused across different endpoints
 */
export function createSSEStream<T extends SSEEvent>(
  generator: AsyncGenerator<T>,
  options?: {
    onError?: (error: Error) => SSEEvent
    onComplete?: () => SSEEvent
    headers?: Record<string, string>
  },
): Response {
  const headers = {
    'Content-Type': 'text/event-stream',
    'Cache-Control': 'no-cache',
    Connection: 'keep-alive',
    'X-Accel-Buffering': 'no',
    ...options?.headers,
  }

  const readable = new ReadableStream({
    async start(controller) {
      const encoder = new TextEncoder()

      const sendEvent = (event: string, data: any) => {
        const message = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`
        controller.enqueue(encoder.encode(message))
      }

      try {
        for await (const event of generator) {
          sendEvent(event.type, event.data)
        }

        if (options?.onComplete) {
          const completeEvent = options.onComplete()
          sendEvent(completeEvent.type, completeEvent.data)
        }
      } catch (error) {
        if (options?.onError) {
          const errorEvent = options.onError(error as Error)
          sendEvent(errorEvent.type, errorEvent.data)
        } else {
          sendEvent('error', {
            error: error instanceof Error ? error.message : String(error),
          })
        }
      } finally {
        controller.close()
      }
    },
  })

  return new Response(readable, { headers })
}

/**
 * Helper to create SSE response for Hono context
 */
export function streamSSE<T extends SSEEvent>(
  c: Context,
  generator: AsyncGenerator<T>,
  options?: Parameters<typeof createSSEStream>[1],
): Response {
  // Add CORS headers if needed
  const corsHeaders: Record<string, string> = c.req.header('Origin')
    ? {
        'Access-Control-Allow-Origin': c.req.header('Origin') || '*',
        'Access-Control-Allow-Credentials': 'true',
      }
    : {}

  return createSSEStream(generator, {
    ...options,
    headers: {
      ...corsHeaders,
      ...options?.headers,
    },
  })
}
