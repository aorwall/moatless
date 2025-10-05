import * as net from 'node:net'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  findAnyAvailablePort,
  getPort,
  isPortAvailable,
  pingPort,
  waitForPort,
} from '../port-utils.js'

// Mock external dependencies
vi.mock('node:child_process')
vi.mock('node:fs')
vi.mock('node:net')

const mockNet = vi.mocked(net)

describe('port-utils', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('getPort', () => {
    it('should find an available random port in ephemeral range', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn(),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      // Mock socket creation
      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      // Mock port as available (timeout on both IPv4 and IPv6)
      mockSocket.on.mockImplementation((event: string, callback: () => void) => {
        if (event === 'timeout') {
          setTimeout(() => callback(), 10) // Available (timeout)
        }
        return mockSocket
      })

      const port = await getPort()

      // Should be in ephemeral range
      expect(port).toBeGreaterThanOrEqual(49152)
      expect(port).toBeLessThanOrEqual(65535)
    })

    it('should throw error when no ports available after max attempts', async () => {
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

      await expect(getPort()).rejects.toThrow('Could not find available port after 10 attempts')
    })
  })

  describe('isPortAvailable', () => {
    it('should return false when port is in use (connection succeeds)', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: () => void) => {
          if (event === 'connect') {
            setTimeout(() => callback(), 10) // Port in use
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await isPortAvailable(3000)

      expect(result).toBe(false)
    })

    it('should return true when port is available (connection refused)', async () => {
      const mockSocket = {
        setTimeout: vi.fn(),
        on: vi.fn((event: string, callback: (err?: any) => void) => {
          if (event === 'error') {
            setTimeout(() => callback({ code: 'ECONNREFUSED' }), 10) // Port available
          }
          return mockSocket
        }),
        connect: vi.fn(),
        destroy: vi.fn(),
      }

      vi.mocked(net.Socket).mockImplementation(() => mockSocket as any)

      const result = await isPortAvailable(3000)

      expect(result).toBe(true)
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

      const port = await findAnyAvailablePort()

      expect(port).toBe(12345)
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

      const result = await pingPort(3000)

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

      const result = await pingPort(3000)

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

      const result = await waitForPort(3000, 2000)

      expect(result).toBe(true)
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

      const result = await waitForPort(3000, 100)

      expect(result).toBe(false)
    })
  })
})
