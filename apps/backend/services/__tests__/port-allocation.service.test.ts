import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PortAllocationService } from '../port-allocation.service.js'

// Mock all external dependencies
vi.mock('node:fs', () => ({
  promises: {
    mkdir: vi.fn().mockResolvedValue(undefined),
    readFile: vi.fn().mockRejectedValue({ code: 'ENOENT' }),
    writeFile: vi.fn().mockResolvedValue(undefined),
  },
}))

vi.mock('node:path', () => ({
  default: {
    join: vi.fn((...args) => args.join('/')),
  },
}))

vi.mock('node:net', () => ({
  Socket: vi.fn(() => ({
    setTimeout: vi.fn(),
    on: vi.fn((event, callback) => {
      if (event === 'error' || event === 'timeout') {
        setTimeout(() => callback(), 0)
      }
      return this
    }),
    destroy: vi.fn(),
    connect: vi.fn(),
  })),
  createServer: vi.fn(() => ({
    listen: vi.fn((_port, _host, callback) => {
      setTimeout(() => callback(), 0)
    }),
    address: vi.fn(() => ({ port: 50000 })),
    close: vi.fn((callback) => setTimeout(() => callback(), 0)),
    on: vi.fn(),
  })),
}))

// Get the mocked modules
const { promises: mockFs } = await import('node:fs')
const mockPath = (await import('node:path')).default
const mockNet = await import('node:net')

describe('PortAllocationService - Unit Tests', () => {
  let service: PortAllocationService
  let mockDataDir: string

  beforeEach(async () => {
    vi.clearAllMocks()
    mockDataDir = '/mock/data/dir'

    // Reset all mocks
    vi.mocked(mockFs.mkdir).mockResolvedValue(undefined)
    vi.mocked(mockFs.readFile).mockRejectedValue({ code: 'ENOENT' })
    vi.mocked(mockFs.writeFile).mockResolvedValue(undefined)
    vi.mocked(mockPath.join).mockImplementation((...args) => args.join('/'))

    // Reset singleton
    // @ts-ignore - accessing private members for testing
    PortAllocationService.instance = undefined as any
    service = PortAllocationService.getInstance(mockDataDir)
    await service.initialize()
  })

  afterEach(() => {
    delete process.env.DATA_DIR
  })

  describe('Port Range Management', () => {
    it('should allocate preview ports in range 3000-3999', async () => {
      const port = await service.allocatePort('task-1', 'preview')

      expect(port).toBeGreaterThanOrEqual(3000)
      expect(port).toBeLessThanOrEqual(3999)
    })

    it('should allocate worker ports in range 4000-4999', async () => {
      const port = await service.allocatePort('task-1', 'worker')

      expect(port).toBeGreaterThanOrEqual(4000)
      expect(port).toBeLessThanOrEqual(4999)
    })

    it('should not allocate the same port twice', async () => {
      const port1 = await service.allocatePort('task-1', 'preview')
      const port2 = await service.allocatePort('task-2', 'preview')

      expect(port1).not.toBe(port2)
    })
  })

  describe('Port Allocation with PID', () => {
    it('should store PID when provided', async () => {
      const port = await service.allocatePort('task-1', 'preview', 12345)

      expect(port).toBeDefined()
      expect(mockFs.writeFile).toHaveBeenCalledWith(
        expect.stringContaining('port-allocations.json'),
        expect.stringContaining('"pid": 12345'),
      )
    })

    it('should store allocation without PID when not provided', async () => {
      const port = await service.allocatePort('task-1', 'preview')

      expect(port).toBeDefined()
      expect(mockFs.writeFile).toHaveBeenCalledWith(
        expect.stringContaining('port-allocations.json'),
        expect.any(String),
      )

      const writeCall = vi.mocked(mockFs.writeFile).mock.calls[
        vi.mocked(mockFs.writeFile).mock.calls.length - 1
      ]
      const savedData = JSON.parse(writeCall[1] as string)
      expect(savedData[0]).toEqual({
        port,
        taskId: 'task-1',
        type: 'preview',
        allocatedAt: expect.any(String),
      })
    })
  })

  describe('Port Release', () => {
    it('should release individual port', async () => {
      const port = await service.allocatePort('task-1', 'preview')
      const initialWriteCalls = vi.mocked(mockFs.writeFile).mock.calls.length

      await service.releasePort(port)

      expect(mockFs.writeFile).toHaveBeenCalledTimes(initialWriteCalls + 1)
      const releaseCall = vi.mocked(mockFs.writeFile).mock.calls[
        vi.mocked(mockFs.writeFile).mock.calls.length - 1
      ]
      expect(releaseCall[1]).toBe('[]') // Empty array
    })

    it('should release all ports for a task', async () => {
      await service.allocatePort('task-1', 'preview')
      await service.allocatePort('task-1', 'worker')
      await service.allocatePort('task-2', 'preview') // Different task

      await service.releasePortsForTask('task-1')

      const finalCall = vi.mocked(mockFs.writeFile).mock.calls[
        vi.mocked(mockFs.writeFile).mock.calls.length - 1
      ]
      const finalState = JSON.parse(finalCall[1] as string)

      // Should only have task-2's allocation remaining
      expect(finalState).toHaveLength(1)
      expect(finalState[0].taskId).toBe('task-2')
    })

    it('should handle releasing non-existent port gracefully', async () => {
      await expect(service.releasePort(9999)).resolves.not.toThrow()
    })

    it('should handle releasing ports for non-existent task gracefully', async () => {
      await expect(service.releasePortsForTask('non-existent')).resolves.not.toThrow()
    })
  })

  describe('Port Availability Checking', () => {
    it('should find available port when range has free ports', async () => {
      const port = await service.allocatePort('task-1', 'preview')

      expect(port).toBeGreaterThanOrEqual(3000)
      expect(port).toBeLessThanOrEqual(3999)
    })

    it('should reuse existing port for same task and type', async () => {
      const port1 = await service.allocatePort('task-1', 'preview')
      const port2 = await service.allocatePort('task-1', 'preview')

      expect(port1).toBe(port2) // Should reuse existing allocation
    })

    it('should allocate different ports for different tasks', async () => {
      const port1 = await service.allocatePort('task-1', 'preview')
      const port2 = await service.allocatePort('task-2', 'preview')

      expect(port1).not.toBe(port2)
    })
  })

  describe('Persistence', () => {
    it('should save allocations to file', async () => {
      await service.allocatePort('task-1', 'preview', 12345)

      expect(mockFs.mkdir).toHaveBeenCalledWith(mockDataDir, { recursive: true })
      expect(mockFs.writeFile).toHaveBeenCalledWith(
        expect.stringContaining('port-allocations.json'),
        expect.any(String),
      )
    })

    it('should load existing allocations on startup', async () => {
      const existingAllocations = [
        {
          port: 3001,
          taskId: 'existing-task',
          type: 'preview',
          allocatedAt: new Date().toISOString(),
          pid: 54321,
        },
      ]

      vi.mocked(mockFs.readFile).mockResolvedValueOnce(JSON.stringify(existingAllocations))

      // Create new instance to trigger loading
      // @ts-ignore - accessing private members for testing
      PortAllocationService.instance = undefined as any
      const newService = PortAllocationService.getInstance(mockDataDir)
      await newService.initialize()

      // Should not allocate the already-used port
      const port = await newService.allocatePort('task-1', 'preview')
      expect(port).not.toBe(3001)
    })

    it('should handle corrupted allocation file gracefully', async () => {
      vi.mocked(mockFs.readFile).mockResolvedValueOnce('invalid json')

      // @ts-ignore - accessing private members for testing
      PortAllocationService.instance = undefined as any
      const newService = PortAllocationService.getInstance(mockDataDir)
      await newService.initialize()

      // Should still work despite corrupted file
      await expect(newService.allocatePort('task-1', 'preview')).resolves.toBeDefined()
    })
  })

  describe('Error Handling', () => {
    it('should handle file system errors during save', async () => {
      vi.mocked(mockFs.writeFile).mockRejectedValueOnce(new Error('Disk full'))

      // The service continues to work but logs error
      const port = await service.allocatePort('task-new', 'preview')
      expect(port).toBeDefined()
    })

    it('should handle mkdir errors during initialization', async () => {
      vi.mocked(mockFs.mkdir).mockRejectedValueOnce(new Error('Permission denied'))

      // @ts-ignore - accessing private members for testing
      PortAllocationService.instance = undefined as any
      const newService = PortAllocationService.getInstance(mockDataDir)

      await expect(newService.initialize()).rejects.toThrow('Permission denied')
    })
  })

  describe('Singleton Pattern', () => {
    it('should return same instance on multiple calls', () => {
      const instance1 = PortAllocationService.getInstance()
      const instance2 = PortAllocationService.getInstance()

      expect(instance1).toBe(instance2)
    })
  })

  describe('Port Range Exhaustion', () => {
    it('should throw error when all ports in range are allocated', async () => {
      // Mock scenario where all ports are taken and still in use
      const fullAllocations = []
      for (let port = 3000; port <= 3999; port++) {
        fullAllocations.push({
          port,
          taskId: `task-${port}`,
          type: 'preview',
          allocatedAt: new Date().toISOString(), // Recent allocation
        })
      }

      vi.mocked(mockFs.readFile).mockResolvedValueOnce(JSON.stringify(fullAllocations))

      // Mock port availability to return false (ports are in use)
      const mockSocketInstance = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'connect') {
            // Simulate port in use by triggering connect
            setTimeout(() => callback(), 0)
          }
          return mockSocketInstance
        }),
        destroy: vi.fn(),
        connect: vi.fn(),
      } as any

      vi.mocked(mockNet.Socket).mockImplementation(() => mockSocketInstance)

      // @ts-ignore - accessing private members for testing
      PortAllocationService.instance = undefined as any
      const newService = PortAllocationService.getInstance(mockDataDir)
      await newService.initialize()

      await expect(newService.allocatePort('new-task', 'preview')).rejects.toThrow(
        'No available ports in range 3000-3999',
      )
    })
  })
})
