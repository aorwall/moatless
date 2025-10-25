import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp } from '../app.js'
import { appContainer } from '../container/container.js'
import { sseManager } from '../features/events/index.js'

describe('Graceful Shutdown', () => {
  let mockController: any

  beforeEach(async () => {
    await createApp()

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
      const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined)

      // Run graceful shutdown - should not throw
      await expect(sseManager.gracefulShutdown()).resolves.not.toThrow()

      // Verify error was logged
      expect(consoleErrorSpy).toHaveBeenCalled()

      // Cleanup
      consoleErrorSpy.mockRestore()
    })
  })

  describe('Workspace Service', () => {
    it('should have gracefulShutdown method available', async () => {
      const { WorkspaceService } = await import('../services/workspace.service.js')
      const workspaceService = appContainer.get(WorkspaceService)

      expect(workspaceService.gracefulShutdown).toBeDefined()
      expect(typeof workspaceService.gracefulShutdown).toBe('function')
    })

    it('should cleanup active workspaces during graceful shutdown', async () => {
      const { WorkspaceService } = await import('../services/workspace.service.js')
      const workspaceService = appContainer.get(WorkspaceService)

      // Mock cleanup method to verify it's called
      const cleanupSpy = vi.spyOn(workspaceService, 'cleanupWorkspace').mockResolvedValue()

      // Add a mock workspace to active workspaces
      const mockWorkspace = {
        getStatus: vi.fn().mockReturnValue('idle'),
        teardown: vi.fn().mockResolvedValue(undefined),
        type: 'local',
      } as any

      // Access private field for testing
      ;(workspaceService as any).activeWorkspaces.set('test-task-1', mockWorkspace)

      // Run graceful shutdown
      await workspaceService.gracefulShutdown()

      // Verify cleanup was called
      expect(cleanupSpy).toHaveBeenCalledWith('test-task-1')

      // Cleanup
      cleanupSpy.mockRestore()
    })
  })
})
