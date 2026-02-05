import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SSEConnectionManager } from '../sse-manager'

describe('SSEConnectionManager', () => {
  let manager: SSEConnectionManager
  let mockController: any

  beforeEach(() => {
    manager = new SSEConnectionManager()
    mockController = {
      enqueue: vi.fn(),
      close: vi.fn(),
      error: vi.fn(),
      desiredSize: 1,
    }
  })

  afterEach(() => {
    // Clean up any connections
    manager.gracefulShutdown()
  })

  describe('Task-specific connections', () => {
    it('should connect and track task connections', () => {
      const taskId = 'task-123'
      const connectionId = manager.connect(taskId, mockController)

      expect(connectionId).toMatch(/^sse_\d+$/)
      expect(manager.getConnectionCount(taskId)).toBe(1)
      expect(manager.hasConnections(taskId)).toBe(true)
    })

    it('should support multiple connections per task', () => {
      const taskId = 'task-123'
      const mockController2 = { ...mockController }

      manager.connect(taskId, mockController)
      manager.connect(taskId, mockController2)

      expect(manager.getConnectionCount(taskId)).toBe(2)
    })

    it('should disconnect task connections', () => {
      const taskId = 'task-123'
      const connectionId = manager.connect(taskId, mockController)

      manager.disconnect(taskId, connectionId)

      expect(manager.getConnectionCount(taskId)).toBe(0)
      expect(manager.hasConnections(taskId)).toBe(false)
    })

    it('should preserve lastEventId when provided', () => {
      const taskId = 'task-123'
      const lastEventId = 'event-456'
      manager.connect(taskId, mockController, lastEventId)

      const details = manager.getTaskConnectionDetails(taskId)
      expect(details.connectionCount).toBe(1)
    })

    it('should return all task IDs with connections', () => {
      manager.connect('task-1', mockController)
      manager.connect('task-2', { ...mockController })

      const taskIds = manager.getAllTaskIds()

      expect(taskIds).toContain('task-1')
      expect(taskIds).toContain('task-2')
      expect(taskIds).toHaveLength(2)
    })

    it('should get task connection details with event counts', () => {
      const taskId = 'task-123'
      manager.connect(taskId, mockController)

      const details = manager.getTaskConnectionDetails(taskId)

      expect(details).toEqual({
        connectionCount: 1,
        totalEvents: 0,
      })
    })
  })

  describe('Connection limits', () => {
    it('should reject task connections when limit is reached', () => {
      const taskId = 'task-limit'
      // Create 100 connections (the limit)
      for (let i = 0; i < 100; i++) {
        manager.connect(taskId, { ...mockController })
      }

      expect(manager.getConnectionCount(taskId)).toBe(100)

      // The 101st connection should throw
      expect(() => manager.connect(taskId, mockController)).toThrow(
        'Connection limit reached for task',
      )
    })

    it('should reject global connections when limit is reached', () => {
      // Create 500 connections (the limit)
      for (let i = 0; i < 500; i++) {
        manager.connectGlobal(`global-${i}`, { ...mockController })
      }

      expect(manager.getGlobalConnectionCount()).toBe(500)

      // The 501st connection should throw
      expect(() => manager.connectGlobal('global-overflow', mockController)).toThrow(
        'Connection limit reached',
      )
    })

    it('should allow connections after disconnecting when at limit', () => {
      const taskId = 'task-reconnect'
      const connectionIds: string[] = []

      // Fill up to limit
      for (let i = 0; i < 100; i++) {
        connectionIds.push(manager.connect(taskId, { ...mockController }))
      }

      // Disconnect one
      manager.disconnect(taskId, connectionIds[0] as string)

      // Should now allow a new connection
      expect(() => manager.connect(taskId, mockController)).not.toThrow()
      expect(manager.getConnectionCount(taskId)).toBe(100)
    })
  })

  describe('Global connections', () => {
    it('should connect global connections', () => {
      const connectionId = 'global-123'
      manager.connectGlobal(connectionId, mockController)

      expect(manager.getGlobalConnectionCount()).toBe(1)
      expect(manager.getGlobalConnections()).toHaveLength(1)
    })

    it('should disconnect global connections', () => {
      const connectionId = 'global-123'
      manager.connectGlobal(connectionId, mockController)
      manager.disconnectGlobal(connectionId)

      expect(manager.getGlobalConnectionCount()).toBe(0)
    })

    it('should increment global event count', () => {
      manager.connectGlobal('global-1', mockController)

      manager.incrementGlobalEventCount()

      const details = manager.getGlobalConnectionDetails()
      expect(details.totalEvents).toBe(1)
    })

    it('should track multiple global connections', () => {
      manager.connectGlobal('global-1', mockController)
      manager.connectGlobal('global-2', { ...mockController })

      manager.incrementGlobalEventCount()

      const details = manager.getGlobalConnectionDetails()
      expect(details.connectionCount).toBe(2)
      expect(details.totalEvents).toBe(2) // Each connection gets incremented
    })
  })

  describe('getAllConnectionStats', () => {
    it('should return comprehensive stats', () => {
      // Setup task connections
      manager.connect('task-1', mockController)
      manager.connect('task-2', { ...mockController })

      // Setup global connections
      manager.connectGlobal('global-1', { ...mockController })

      const stats = manager.getAllConnectionStats()

      expect(stats.global.connectionCount).toBe(1)
      expect(stats.tasks?.['task-1']?.connectionCount).toBe(1)
      expect(stats.tasks?.['task-2']?.connectionCount).toBe(1)
      expect(stats.total.connectionCount).toBe(3) // 2 task + 1 global
    })
  })

  describe('getAllTaskConnectionCounts', () => {
    it('should return connection counts for all tasks', () => {
      manager.connect('task-1', mockController)
      manager.connect('task-1', { ...mockController })
      manager.connect('task-2', { ...mockController })

      const counts = manager.getAllTaskConnectionCounts()

      expect(counts?.['task-1']).toBe(2)
      expect(counts?.['task-2']).toBe(1)
    })
  })

  describe('gracefulShutdown', () => {
    it('should close all task connections', () => {
      const mockController1 = {
        enqueue: vi.fn(),
        close: vi.fn(),
        error: vi.fn(),
        desiredSize: 1,
      }
      const mockController2 = {
        enqueue: vi.fn(),
        close: vi.fn(),
        error: vi.fn(),
        desiredSize: 1,
      }

      manager.connect('task-1', mockController1)
      manager.connect('task-2', mockController2)

      manager.gracefulShutdown()

      expect(mockController1.enqueue).toHaveBeenCalled()
      expect(mockController1.close).toHaveBeenCalled()
      expect(mockController2.enqueue).toHaveBeenCalled()
      expect(mockController2.close).toHaveBeenCalled()
      expect(manager.getAllTaskIds()).toHaveLength(0)
    })

    it('should close all global connections', () => {
      const mockGlobalController = {
        enqueue: vi.fn(),
        close: vi.fn(),
        error: vi.fn(),
        desiredSize: 1,
      }

      manager.connectGlobal('global-1', mockGlobalController)

      manager.gracefulShutdown()

      expect(mockGlobalController.enqueue).toHaveBeenCalled()
      expect(mockGlobalController.close).toHaveBeenCalled()
      expect(manager.getGlobalConnectionCount()).toBe(0)
    })

    it('should send close event to clients', () => {
      manager.connectGlobal('global-1', mockController)

      manager.gracefulShutdown()

      expect(mockController.enqueue).toHaveBeenCalledWith(expect.stringContaining('"type":"close"'))
      expect(mockController.enqueue).toHaveBeenCalledWith(
        expect.stringContaining('"reason":"server_shutdown"'),
      )
    })

    it('should handle errors during shutdown gracefully', () => {
      const errorController = {
        enqueue: vi.fn().mockImplementation(() => {
          throw new Error('Connection error')
        }),
        close: vi.fn(),
        error: vi.fn(),
        desiredSize: 1,
      }

      manager.connectGlobal('global-1', errorController)

      // Should not throw
      expect(() => manager.gracefulShutdown()).not.toThrow()
    })
  })
})
