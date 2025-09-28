import * as net from 'node:net'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PortService } from '../port.service.js'

// Mock external dependencies
vi.mock('node:child_process')
vi.mock('node:fs')
vi.mock('node:net')

const mockNet = vi.mocked(net)

describe('PortService', () => {
  let portService: PortService
  let mockLogger: any

  beforeEach(() => {
    vi.clearAllMocks()

    mockLogger = {
      debug: vi.fn(),
      info: vi.fn(),
      warn: vi.fn(),
      error: vi.fn(),
    }

    portService = new PortService(mockLogger)
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('findAvailablePort', () => {
    it('should find first available port in specified range', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn(),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      // Mock socket creation
      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      // Mock first port (9000) as unavailable, second port (9001) as available
      mockSocket.on
        .mockImplementationOnce((event: string, callback: () => void) => {
          if (event === 'connect') {
            setTimeout(() => callback(), 10) // Port 9000 in use
          }
          return mockSocket
        })
        .mockImplementationOnce((event: string, _callback: () => void) => {
          if (event === 'timeout') return mockSocket
          if (event === 'error') return mockSocket
          return mockSocket
        })
        .mockImplementationOnce((event: string, _callback: () => void) => {
          if (event === 'timeout') return mockSocket
          if (event === 'error') return mockSocket
          return mockSocket
        })
        .mockImplementationOnce((event: string, callback: () => void) => {
          if (event === 'timeout') {
            setTimeout(() => callback(), 10) // Port 9001 available (timeout)
          }
          return mockSocket
        })
        .mockImplementationOnce((event: string, _callback: () => void) => {
          if (event === 'connect') return mockSocket
          if (event === 'error') return mockSocket
          return mockSocket
        })
        .mockImplementationOnce((event: string, _callback: () => void) => {
          if (event === 'connect') return mockSocket
          if (event === 'timeout') return mockSocket
          return mockSocket
        })

      const port = await portService.findAvailablePort(9000)

      expect(port).toBe(9001)
      expect(mockLogger.debug).toHaveBeenCalledWith('Found and reserved available port 9001')
    })

    it('should throw error when no ports available in range', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'connect') {
            setTimeout(() => callback(), 10) // All ports in use
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      await expect(portService.findAvailablePort(9000)).rejects.toThrow(
        'No available ports in range 9000-9999',
      )
    })
  })

  describe('findAnyAvailablePort', () => {
    it('should find any available port using OS', async () => {
      const mockServer = {
        listen: vi.fn((_port: number, _host: string, callback: () => void) => {
          setTimeout(() => callback(), 10)
        }),
        address: vi.fn(() => ({ port: 12345 })),
        close: vi.fn((callback: () => void) => {
          setTimeout(() => callback(), 10)
        }),
        on: vi.fn(),
      }

      mockNet.createServer.mockReturnValue(mockServer as any)

      const port = await portService.findAnyAvailablePort()

      expect(port).toBe(12345)
      expect(mockLogger.debug).toHaveBeenCalledWith('Found any available port: 12345')
    })
  })

  describe('pingPort', () => {
    it('should return true when port is responsive', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'connect') {
            setTimeout(() => callback(), 10)
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await portService.pingPort(3000)

      expect(result).toBe(true)
    })

    it('should return false when port times out', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'timeout') {
            setTimeout(() => callback(), 10)
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await portService.pingPort(3000)

      expect(result).toBe(false)
    })
  })

  describe('waitForPort', () => {
    it('should return true when port becomes responsive', async () => {
      let callCount = 0
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'connect' && callCount > 0) {
            setTimeout(() => callback(), 10) // Responsive after first attempt
          } else if (event === 'timeout' && callCount === 0) {
            setTimeout(() => callback(), 10) // Not responsive on first attempt
          }
          return mockSocket
        }),
        connect: vi.fn(() => {
          callCount++
        }),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await portService.waitForPort(3000, 2000)

      expect(result).toBe(true)
      expect(mockLogger.debug).toHaveBeenCalledWith('Waiting for port 3000 to become responsive...')
    })

    it('should return false when port does not become responsive within timeout', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'timeout') {
            setTimeout(() => callback(), 10) // Always timeout
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await portService.waitForPort(3000, 100)

      expect(result).toBe(false)
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'Port 3000 did not become responsive within 100ms',
      )
    })
  })
})
