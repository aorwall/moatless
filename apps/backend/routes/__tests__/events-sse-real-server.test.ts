import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { FastifyInstance } from 'fastify'
import { EventType } from '@moatless/shared/types/event'
import { build } from '../../test/helper.js'

/**
 * SSE Real Server Integration Test
 * 
 * Verifies that Server-Sent Events (SSE) work correctly with real HTTP connections.
 * This test starts an actual server and establishes real SSE connections to ensure
 * the event streaming mechanism works properly.
 * 
 * The SSE endpoint now streams all events by default without filtering by task status.
 */
describe('SSE Real Server Integration', () => {
  let app: FastifyInstance
  let serverUrl: string

  beforeEach(async () => {
    app = await build()
    
    // Start the server on a random port
    await app.listen({ port: 0, host: '127.0.0.1' })
    const address = app.server.address() as { port: number }
    serverUrl = `http://127.0.0.1:${address.port}`
  })

  afterEach(async () => {
    await app.close()
  })

  it('should receive events through SSE stream', async () => {
    console.log('🧪 Testing SSE with real HTTP connection...')
    
    // Connect to SSE stream (now streams all events by default)
    const abortController = new AbortController()
    const sseResponse = await fetch(`${serverUrl}/api/v1/events/stream`, {
      signal: abortController.signal,
      headers: {
        'Accept': 'text/event-stream',
      },
    })
    
    expect(sseResponse.ok).toBe(true)
    expect(sseResponse.headers.get('content-type')).toBe('text/event-stream')
    console.log('✅ Connected to SSE stream')
    
    // Create a reader to read the stream
    const reader = sseResponse.body!.getReader()
    const decoder = new TextDecoder()
    
    // Track received data
    const receivedData: string[] = []
    let receivedTestEvent = false
    
    // Start reading stream
    const readPromise = (async () => {
      let buffer = ''
      try {
        while (true) {
          const { done, value } = await reader.read()
          if (done) break
          
          const chunk = decoder.decode(value, { stream: true })
          buffer += chunk
          
          // Process complete lines
          const lines = buffer.split('\n')
          buffer = lines.pop() || ''
          
          for (const line of lines) {
            if (line.trim()) {
              console.log('📨 Received line:', line.substring(0, 100) + (line.length > 100 ? '...' : ''))
              receivedData.push(line)
              
              // Check if this is our test event
              try {
                const data = JSON.parse(line)
                if (data.eventId === 'sse-test-minimal') {
                  receivedTestEvent = true
                  console.log('✅ Received our test event!')
                }
              } catch {
                // Not JSON, might be ping or other data
              }
            }
          }
        }
      } catch (error: any) {
        if (error.name !== 'AbortError') {
          console.error('Stream error:', error)
        }
      }
    })()
    
    // Wait a bit for connection to establish
    await new Promise(resolve => setTimeout(resolve, 200))
    
    // Emit a test event directly through the event service
    console.log('📤 Emitting test event...')
    await app.services.event.emitEvent({
      eventId: 'sse-test-minimal',
      taskId: 'test-task-minimal',
      type: EventType.MESSAGE,
      timestamp: new Date(),
      agentType: 'test-agent',
      isError: false,
      payload: { content: 'This is a test event for SSE' },
    })
    
    // Wait for the event to be received
    console.log('⏳ Waiting for event to be received...')
    await new Promise(resolve => setTimeout(resolve, 1000))
    
    // Clean up
    abortController.abort()
    await readPromise
    
    // Verify results
    console.log(`📊 Received ${receivedData.length} lines of data`)
    expect(receivedData.length).toBeGreaterThan(0)
    expect(receivedTestEvent).toBe(true)
    
    // Verify we received at least one valid JSON event
    let foundValidEvent = false
    for (const line of receivedData) {
      try {
        const data = JSON.parse(line)
        if (data.eventId || data.type) {
          foundValidEvent = true
          break
        }
      } catch {
        // Skip non-JSON lines
      }
    }
    expect(foundValidEvent).toBe(true)
    
    console.log('✅ SSE test passed!')
  }, 10000)
})