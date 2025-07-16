import { EventType } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

/**
 * SSE Event Subscription Test
 *
 * Verifies that the global event subscription mechanism works correctly
 * for Server-Sent Events (SSE) streaming.
 */
describe('SSE Event Subscription', () => {
  let app: any

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {})

  it('should emit and receive events through subscribeAll', async () => {
    const eventService = app.services.event

    // Track received events
    const receivedEvents: any[] = []

    // Subscribe using subscribeAll for global event stream
    const unsubscribe = eventService.subscribeAll((event: any) => {
      receivedEvents.push(event)
    })

    // Emit multiple events
    for (let i = 1; i <= 3; i++) {
      await eventService.emitEvent({
        eventId: `simple-test-${i}`,
        taskId: 'simple-task',
        type: EventType.MESSAGE,
        timestamp: new Date(),
        agentType: 'test-agent',
        isError: false,
        payload: { content: `Message ${i}` },
      })
    }

    // Wait for async operations
    await new Promise((resolve) => setTimeout(resolve, 50))

    // Cleanup
    unsubscribe()

    // Verify all events were received
    expect(receivedEvents.length).toBe(3)
    expect(receivedEvents[0].eventId).toBe('simple-test-1')
    expect(receivedEvents[1].eventId).toBe('simple-test-2')
    expect(receivedEvents[2].eventId).toBe('simple-test-3')
  })

  it('should handle both task-specific and global subscriptions', async () => {
    const eventService = app.services.event
    const taskId = 'specific-task'

    // Track events for specific task
    const taskEvents: any[] = []
    const unsubscribeTask = eventService.subscribe(taskId, (event: any) => {
      taskEvents.push(event)
    })

    // Track all events
    const allEvents: any[] = []
    const unsubscribeAll = eventService.subscribeAll((event: any) => {
      allEvents.push(event)
    })

    // Emit event for specific task
    await eventService.emitEvent({
      eventId: 'task-specific-1',
      taskId,
      type: EventType.START,
      timestamp: new Date(),
      agentType: 'test-agent',
      isError: false,
      payload: {},
    })

    // Emit event for different task
    await eventService.emitEvent({
      eventId: 'other-task-1',
      taskId: 'other-task',
      type: EventType.START,
      timestamp: new Date(),
      agentType: 'test-agent',
      isError: false,
      payload: {},
    })

    // Wait
    await new Promise((resolve) => setTimeout(resolve, 50))

    // Cleanup
    unsubscribeTask()
    unsubscribeAll()

    // Task-specific should only receive its event
    expect(taskEvents.length).toBe(1)
    expect(taskEvents[0].eventId).toBe('task-specific-1')

    // Global should receive both
    expect(allEvents.length).toBe(2)
    expect(allEvents.map((e) => e.eventId)).toContain('task-specific-1')
    expect(allEvents.map((e) => e.eventId)).toContain('other-task-1')
  })
})
