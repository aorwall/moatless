import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import WebSocket from 'ws'
import { build } from '../../test/helper.js'

describe('WebSocket Events', () => {
  let app: FastifyInstance
  let address: any

  beforeEach(async () => {
    app = await build()
    await app.listen({ port: 0 }) // Random port
    address = app.server.address()
  })

  afterEach(async () => {
    await app.close()
  })

  it('should stream task events via WebSocket', async () => {
    const taskId = 'test-task-123'
    const mockTask = {
      taskId,
      status: 'active',
      createdAt: new Date(),
      lastMessageAt: new Date(),
      agentType: 'claude-code',
      environmentId: 'test-env',
      numTurns: 0,
      totalCostUsd: 0,
      iterations: [],
    }

    // Mock task exists
    vi.spyOn(app.services.task, 'getTask').mockResolvedValue(mockTask)

    // Connect to WebSocket
    const port = typeof address === 'string' ? address : address?.port
    const ws = new WebSocket(`ws://localhost:${port}/api/tasks/${taskId}/events/stream`)

    const messages: any[] = []

    ws.on('message', (data) => {
      messages.push(JSON.parse(data.toString()))
    })

    // Wait for connection
    await new Promise<void>((resolve, reject) => {
      ws.on('open', () => resolve())
      ws.on('error', reject)
      setTimeout(() => reject(new Error('Connection timeout')), 1000)
    })

    // Should receive connection event
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(messages).toHaveLength(1)
    expect(messages[0].type).toBe('connection')
    expect(messages[0].status).toBe('connected')
    expect(messages[0].taskId).toBe(taskId)

    // Emit an event to the task
    await app.services.event.emitEvent({
      eventId: 'evt-1',
      type: 'message' as any,
      taskId,
      timestamp: new Date(),
      isError: false,
      payload: { content: 'Test message' },
    })

    // Wait for event to be received
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(messages).toHaveLength(2)
    expect(messages[1].type).toBe('message')
    expect(messages[1].payload.content).toBe('Test message')

    ws.close()
  })

  it('should reject connection for non-existent task', async () => {
    // Mock task not found
    vi.spyOn(app.services.task, 'getTask').mockResolvedValue(null)

    const port = typeof address === 'string' ? address : address?.port
    const ws = new WebSocket(`ws://localhost:${port}/api/tasks/non-existent/events/stream`)

    await new Promise<void>((resolve) => {
      ws.on('close', (code, reason) => {
        expect(code).toBe(1008)
        expect(reason.toString()).toBe('Task not found')
        resolve()
      })
    })
  })

  it('should clean up subscription on disconnect', async () => {
    const taskId = 'test-task-456'
    const mockTask = {
      taskId,
      status: 'active',
    } as any

    vi.spyOn(app.services.task, 'getTask').mockResolvedValue(mockTask)

    const unsubscribeSpy = vi.fn()
    vi.spyOn(app.services.event, 'subscribe').mockReturnValue(unsubscribeSpy)

    const port = typeof address === 'string' ? address : address?.port
    const ws = new WebSocket(`ws://localhost:${port}/api/tasks/${taskId}/events/stream`)

    // Wait for connection
    await new Promise<void>((resolve, reject) => {
      ws.on('open', () => resolve())
      ws.on('error', reject)
      setTimeout(() => reject(new Error('Connection timeout')), 1000)
    })

    // Close connection
    ws.close()

    // Wait for cleanup
    await new Promise((resolve) => setTimeout(resolve, 50))

    // Verify unsubscribe was called
    expect(unsubscribeSpy).toHaveBeenCalled()
  })
})
