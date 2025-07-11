import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp } from '../app.js'
import { sseManager } from '../utils/sse-manager.js'

describe('Graceful Shutdown', () => {
  let appWithServices: Awaited<ReturnType<typeof createApp>>
  let mockController: any

  beforeEach(async () => {
    appWithServices = await createApp()

    // Mock ReadableStreamDefaultController
    mockController = {
      enqueue: vi.fn(),
      close: vi.fn(),
    }
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('SSE Connection Manager', () => {
    it('should close all task connections during graceful shutdown', async () => {
      // Add some test connections
      sseManager.connect('task-1', mockController, 'last-event-1')
      sseManager.connect('task-1', mockController, 'last-event-2')
      sseManager.connect('task-2', mockController, 'last-event-3')

      // Verify connections exist
      expect(sseManager.getConnectionCount('task-1')).toBe(2)
      expect(sseManager.getConnectionCount('task-2')).toBe(1)

      // Run graceful shutdown
      await sseManager.gracefulShutdown()

      // Verify connections were closed
      expect(sseManager.getConnectionCount('task-1')).toBe(0)
      expect(sseManager.getConnectionCount('task-2')).toBe(0)

      // Verify close event was sent
      expect(mockController.enqueue).toHaveBeenCalled()
      expect(mockController.close).toHaveBeenCalled()
    })

    it('should close all global connections during graceful shutdown', async () => {
      // Add global connections
      sseManager.connectGlobal('global-1', mockController)
      sseManager.connectGlobal('global-2', mockController)

      // Verify connections exist
      expect(sseManager.getGlobalConnectionCount()).toBe(2)

      // Run graceful shutdown
      await sseManager.gracefulShutdown()

      // Verify connections were closed
      expect(sseManager.getGlobalConnectionCount()).toBe(0)

      // Verify close event was sent and connections closed
      expect(mockController.enqueue).toHaveBeenCalled()
      expect(mockController.close).toHaveBeenCalled()
    })

    it('should handle errors gracefully when closing connections', async () => {
      // Mock controller that throws error
      const errorController = {
        enqueue: vi.fn().mockImplementation(() => {
          throw new Error('Enqueue error')
        }),
        close: vi.fn(),
        desiredSize: null,
        error: vi.fn(),
      } as unknown as ReadableStreamDefaultController

      // Add connection with error-throwing controller
      sseManager.connect('task-error', errorController)

      // Spy on console.error
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})

      // Run graceful shutdown - should not throw
      await expect(sseManager.gracefulShutdown()).resolves.not.toThrow()

      // Verify error was logged
      expect(consoleErrorSpy).toHaveBeenCalled()

      // Cleanup
      consoleErrorSpy.mockRestore()
    })
  })

  describe('Preview Environment Service', () => {
    it('should have gracefulShutdown method available', () => {
      expect(appWithServices.services.previewEnvironment.gracefulShutdown).toBeDefined()
      expect(typeof appWithServices.services.previewEnvironment.gracefulShutdown).toBe('function')
    })
  })
})
