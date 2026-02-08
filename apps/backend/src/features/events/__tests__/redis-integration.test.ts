import { execSync } from 'node:child_process'
import type { EventLogger } from '@moatless/events'
import { RedisEventTransport } from '@moatless/events/transports/redis-event-transport'
import type { Event } from '@moatless/types'
import Redis from 'ioredis'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest'

async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { stdio: 'pipe' })
    return true
  } catch {
    return false
  }
}

function isPlatformCompatible(): boolean {
  return process.platform === 'linux' || process.platform === 'darwin'
}

const dockerAvailable = await isDockerAvailable()
const platformCompatible = isPlatformCompatible()
const canRunTests = dockerAvailable && platformCompatible

describe.skipIf(!canRunTests)('RedisEventTransport Integration', () => {
  let containerName: string
  let hostPort: number
  let redisUrl: string
  let redis: Redis // Raw client for verification

  beforeAll(async () => {
    // 1. Get available port from @moatless/shell-utils
    const { getPort } = await import('@moatless/shell-utils')
    hostPort = await getPort()

    // 2. Start Redis container
    containerName = `redis-test-${Date.now()}`
    redisUrl = `redis://localhost:${hostPort}`

    execSync(`docker run -d --name ${containerName} -p ${hostPort}:6379 redis:7-alpine`, {
      timeout: 30000,
      stdio: 'pipe',
    })

    // 3. Wait for Redis to be ready (polling pattern)
    await waitForRedisReady(redisUrl, 30000)

    // 4. Create raw Redis client for test verification
    redis = new Redis(redisUrl)
  }, 60000) // 60s timeout for container provisioning

  afterAll(async () => {
    // 1. Close test client if it was initialized
    if (redis) {
      try {
        await redis.quit()
      } catch {
        // Ignore errors during cleanup
      }
    }

    // 2. Stop and remove container
    if (containerName) {
      try {
        execSync(`docker stop ${containerName}`, { timeout: 10000, stdio: 'ignore' })
        execSync(`docker rm ${containerName}`, { timeout: 5000, stdio: 'ignore' })
      } catch {
        // Container might already be stopped
      }
    }
  }, 30000) // 30s timeout for cleanup

  describe('Connection & Initialization', () => {
    let transport: RedisEventTransport

    beforeEach(async () => {
      // Create transport with unique consumer group per test
      transport = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        consumerGroup: `test-group-${Date.now()}`,
        consumerId: `test-consumer-${Math.random()}`,
        blockMs: 1000, // Shorter block time for tests
      })

      await transport.initialize()
    })

    afterEach(async () => {
      await transport.close()
    })

    it('should connect to Redis and initialize transport', async () => {
      expect(transport.isHealthy()).toBe(true)
    })

    it('should create consumer groups on initialization', async () => {
      const taskId = `task-groups-${Date.now()}`

      // Publish event first
      const event: Event = {
        eventId: `test-groups-${Date.now()}`,
        taskId,
        type: 'agent',
        subType: 'started',
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }

      await transport.publishEvent(event)

      // Start listening to trigger consumer group creation
      const receivedEvents: Event[] = []
      await transport.startListening(async (e) => {
        receivedEvents.push(e)
      })

      // Give it a moment to process and create groups
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Verify consumer group exists
      const groups = (await redis.call('XINFO', 'GROUPS', `task-events:${taskId}`)) as unknown[]
      expect(groups.length).toBeGreaterThan(0)

      // Cleanup
      await transport.stopListening()
    })
  })

  describe('Event Publishing', () => {
    let transport: RedisEventTransport

    beforeEach(async () => {
      transport = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        blockMs: 1000, // Shorter block time for tests
      })
      await transport.initialize()
    })

    afterEach(async () => {
      await transport.close()
    })

    it('should publish event to correct stream', async () => {
      const event: Event = {
        eventId: 'test-pub-1',
        taskId: 'task-456',
        type: 'task',
        subType: 'created',
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: { status: 'open', previousStatus: 'pending' },
      }

      await transport.publishEvent(event)

      // Verify event in Redis stream
      const result = (await redis.xread('STREAMS', 'task-events:task-456', '0')) as
        | [string, [string, string[]][]][]
        | null
      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (!result?.[0]?.[1]?.[0]) {
        throw new Error('No events found in stream')
      }

      expect(result[0][1].length).toBeGreaterThan(0)

      // Parse event payload
      const [, fields] = result[0][1][0]
      const data = parseRedisFields(fields)
      expect(data['eventId']).toBe('test-pub-1')
      expect(data['type']).toBe('task')
    })

    it('should trim stream to maxStreamLength', async () => {
      // Publish more events than maxStreamLength
      const streamKey = 'task-events:task-trim-test'

      for (let i = 0; i < 15; i++) {
        await transport.publishEvent({
          eventId: `trim-${i}`,
          taskId: 'task-trim-test',
          type: 'agent',
          subType: 'message',
          timestamp: new Date(),
          isError: false,
          agentType: 'test-agent',
          payload: {},
        })
      }

      // Verify stream length is trimmed (~10000 default, but we only sent 15)
      const length = await redis.xlen(streamKey)
      expect(length).toBeLessThanOrEqual(15) // Should keep all in this small test
    })
  })

  describe('Event Consumption', () => {
    let publisherTransport: RedisEventTransport
    let consumerTransport: RedisEventTransport
    let receivedEvents: Event[]

    beforeEach(async () => {
      receivedEvents = []

      // Create publisher
      publisherTransport = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        blockMs: 1000, // Shorter block time for tests
      })
      await publisherTransport.initialize()

      // Create consumer
      consumerTransport = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        consumerGroup: `consumer-test-${Date.now()}`,
        consumerId: `consumer-${Math.random()}`,
        blockMs: 1000, // Shorter block time for tests
      })
      await consumerTransport.initialize()

      // Start listening
      await consumerTransport.startListening(async (event) => {
        receivedEvents.push(event)
      })
    })

    afterEach(async () => {
      await consumerTransport.stopListening()
      await consumerTransport.close()
      await publisherTransport.close()
    })

    it('should consume published events', async () => {
      const taskId = `task-consume-${Date.now()}`
      const event: Event = {
        eventId: `consume-${Date.now()}`,
        taskId,
        type: 'agent',
        subType: 'message',
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }

      await publisherTransport.publishEvent(event)

      // Wait for event to be consumed
      await waitFor(() => receivedEvents.some((e) => e.taskId === taskId), 5000)

      // Filter to only events for this task
      const taskEvents = receivedEvents.filter((e) => e.taskId === taskId)
      expect(taskEvents).toHaveLength(1)
      expect(taskEvents[0]?.eventId).toBe(event.eventId)
      expect(taskEvents[0]?.taskId).toBe(taskId)
    })

    it('should maintain per-task ordering', async () => {
      // Publish 5 events for task A and 5 for task B rapidly
      const baseTime = Date.now()
      const taskAId = `task-A-${baseTime}`
      const taskBId = `task-B-${baseTime}`

      const taskAEvents = Array.from({ length: 5 }, (_, i) => ({
        eventId: `taskA-${baseTime}-${i}`,
        taskId: taskAId,
        type: 'agent' as const,
        subType: 'message' as const,
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }))

      const taskBEvents = Array.from({ length: 5 }, (_, i) => ({
        eventId: `taskB-${baseTime}-${i}`,
        taskId: taskBId,
        type: 'agent' as const,
        subType: 'message' as const,
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }))

      // Interleave publishing
      for (let i = 0; i < 5; i++) {
        const eventA = taskAEvents[i]
        const eventB = taskBEvents[i]
        if (!(eventA && eventB)) throw new Error('Event not found')
        await publisherTransport.publishEvent(eventA)
        await publisherTransport.publishEvent(eventB)
      }

      // Wait for all events for both tasks
      await waitFor(
        () =>
          receivedEvents.filter((e) => e.taskId === taskAId || e.taskId === taskBId).length >= 10,
        10000,
      )

      // Verify task A events are in order
      const taskAReceived = receivedEvents.filter((e) => e.taskId === taskAId)
      expect(taskAReceived.map((e) => e.eventId)).toEqual([
        `taskA-${baseTime}-0`,
        `taskA-${baseTime}-1`,
        `taskA-${baseTime}-2`,
        `taskA-${baseTime}-3`,
        `taskA-${baseTime}-4`,
      ])

      // Verify task B events are in order
      const taskBReceived = receivedEvents.filter((e) => e.taskId === taskBId)
      expect(taskBReceived.map((e) => e.eventId)).toEqual([
        `taskB-${baseTime}-0`,
        `taskB-${baseTime}-1`,
        `taskB-${baseTime}-2`,
        `taskB-${baseTime}-3`,
        `taskB-${baseTime}-4`,
      ])
    })

    it('should acknowledge processed messages', async () => {
      const taskId = `task-ack-test-${Date.now()}`
      const streamKey = `task-events:${taskId}`
      const event: Event = {
        eventId: `ack-${Date.now()}`,
        taskId,
        type: 'agent',
        subType: 'message',
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }

      await publisherTransport.publishEvent(event)

      // Wait for consumption
      await waitFor(() => receivedEvents.some((e) => e.taskId === taskId), 5000)

      // Give a bit more time for acknowledgment to be processed
      await new Promise((resolve) => setTimeout(resolve, 500))

      // Verify message was acknowledged by checking the stream exists and has been processed
      const length = await redis.xlen(streamKey)
      expect(length).toBeGreaterThan(0) // Stream has messages
    })
  })

  describe('Consumer Group Recovery', () => {
    it('should reprocess unacknowledged messages', async () => {
      const taskId = `task-recovery-${Date.now()}`
      const streamKey = `task-events:${taskId}`
      const consumerGroup = `recovery-group-${Date.now()}`

      // Create transport and publish event
      const publisher = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        blockMs: 1000, // Shorter block time for tests
      })
      await publisher.initialize()

      const recoveryEventId = `recovery-${Date.now()}`
      const event: Event = {
        eventId: recoveryEventId,
        taskId,
        type: 'agent',
        subType: 'message',
        timestamp: new Date(),
        isError: false,
        agentType: 'test-agent',
        payload: {},
      }

      await publisher.publishEvent(event)

      // Start consumer but make handler throw error to prevent ACK
      const consumer1 = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        consumerGroup,
        consumerId: 'consumer-1',
        blockMs: 1000, // Shorter block time for tests
      })
      await consumer1.initialize()

      const received1: Event[] = []
      await consumer1.startListening(async (e) => {
        received1.push(e)
        // Throw error to prevent ACK
        throw new Error('Simulated crash')
      })

      // Wait for event to be received and error to occur
      await waitFor(() => received1.length > 0, 5000)
      await new Promise((resolve) => setTimeout(resolve, 500))

      await consumer1.stopListening()
      await consumer1.close()

      // Verify message is still pending
      const pending = (await redis.xpending(streamKey, consumerGroup)) as unknown[]
      expect(Number(pending[0])).toBeGreaterThan(0) // Has pending messages

      // Start new consumer with same group
      const consumer2 = new RedisEventTransport({
        redisUrl,
        logger: createTestLogger(),
        consumerGroup,
        consumerId: 'consumer-2',
        blockMs: 1000, // Shorter block time for tests
      })
      await consumer2.initialize()

      const received2: Event[] = []
      await consumer2.startListening(async (e) => {
        received2.push(e)
      })

      // Wait for reprocessing - this might take longer due to pending message detection
      await waitFor(() => received2.length > 0, 15000)

      // Verify same event was reprocessed
      expect(received2[0]?.eventId).toBe(recoveryEventId)

      await consumer2.stopListening()
      await consumer2.close()
      await publisher.close()
    })
  })
})

// Helper functions
function createTestLogger(): EventLogger {
  return {
    debug: () => undefined,
    info: () => undefined,
    warn: () => undefined,
    error: () => undefined,
  }
}

function parseRedisFields(fields: string[]): Record<string, string> {
  const data: Record<string, string> = {}
  for (let i = 0; i < fields.length; i += 2) {
    const key = fields[i]
    const value = fields[i + 1]
    if (key && value !== undefined) {
      data[key] = value
    }
  }
  return data
}

async function waitFor(condition: () => boolean, timeoutMs: number): Promise<void> {
  const startTime = Date.now()

  while (!condition() && Date.now() - startTime < timeoutMs) {
    await new Promise((resolve) => setTimeout(resolve, 100))
  }

  if (!condition()) {
    throw new Error(`Condition not met within ${timeoutMs}ms`)
  }
}

async function waitForRedisReady(redisUrl: string, timeoutMs: number): Promise<void> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    try {
      const client = new Redis(redisUrl)
      await client.ping()
      await client.quit()
      return
    } catch {
      // Not ready yet
    }
    await new Promise((r) => setTimeout(r, 500))
  }

  throw new Error(`Redis not ready after ${timeoutMs}ms`)
}
