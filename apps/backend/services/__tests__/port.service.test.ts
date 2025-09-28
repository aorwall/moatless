import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { PortService } from '../port.service.js'

// Mock external dependencies
vi.mock('node:child_process')
vi.mock('node:fs')
vi.mock('node:net')

const mockFs = vi.mocked(fs)
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

  describe('getProcessesUsingPort', () => {
    beforeEach(() => {
      // Mock platform detection
      Object.defineProperty(process, 'platform', {
        value: 'darwin',
        writable: true,
      })
    })

    it('should get PIDs using port on Unix systems', async () => {
      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: '1234\n5678\n',
      })

      // Mock the execAsync import
      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const pids = await portService.getProcessesUsingPort(3000)

      expect(pids).toEqual([1234, 5678])
      expect(mockLogger.debug).toHaveBeenCalledWith('Processes using port 3000:', {
        pids: [1234, 5678],
      })
    })

    it('should return empty array when no processes found', async () => {
      const mockExecAsync = vi.fn().mockRejectedValue(new Error('No processes found'))

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const pids = await portService.getProcessesUsingPort(3000)

      expect(pids).toEqual([])
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'No processes found using port 3000:',
        expect.objectContaining({ error: 'No processes found' }),
      )
    })

    it('should parse Windows netstat output', async () => {
      Object.defineProperty(process, 'platform', { value: 'win32' })

      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: `
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       1234
  TCP    127.0.0.1:3000         127.0.0.1:54321        ESTABLISHED     5678
        `,
      })

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      // Create new instance to use the mocked execAsync
      const windowsPortService = new PortService(mockLogger)
      const pids = await windowsPortService.getProcessesUsingPort(3000)

      expect(pids).toEqual([1234, 5678])
    })
  })

  describe('validatePortBelongsToPid', () => {
    it('should return true when port belongs to PID', async () => {
      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: '1234\n5678\n',
      })

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const result = await portService.validatePortBelongsToPid(3000, 1234)

      expect(result).toBe(true)
      expect(mockLogger.debug).toHaveBeenCalledWith('Port 3000 confirmed to belong to PID 1234')
    })

    it('should return false when port does not belong to PID', async () => {
      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: '5678\n9999\n',
      })

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const result = await portService.validatePortBelongsToPid(3000, 1234)

      expect(result).toBe(false)
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'Port 3000 does not belong to PID 1234 or its children',
        expect.objectContaining({
          port: 3000,
          expectedPid: 1234,
          actualPids: [5678, 9999],
        }),
      )
    })
  })

  describe('detectPortFromLogContent', () => {
    it('should detect port from Vite log pattern', async () => {
      const logContent = `
➜  Local:   http://localhost:3000/
➜  Network: use --host to expose
      `

      const port = await portService.detectPortFromLogContent(logContent)

      expect(port).toBe(3000)
      expect(mockLogger.info).toHaveBeenCalledWith(
        'Detected port 3000 from logs using pattern: /➜\\s+Local:\\s+http:\\/\\/localhost:(\\d+)/gi',
      )
    })

    it('should detect port from generic listening pattern', async () => {
      const logContent = 'Server is listening on port 8080'

      const port = await portService.detectPortFromLogContent(logContent)

      expect(port).toBe(8080)
    })

    it('should return undefined when no port patterns match', async () => {
      const logContent = 'Server started successfully but no port mentioned'

      const port = await portService.detectPortFromLogContent(logContent)

      expect(port).toBeUndefined()
      expect(mockLogger.debug).toHaveBeenCalledWith('No port patterns matched in logs')
    })

    it('should return the most recent port when multiple matches found', async () => {
      const logContent = `
Server started on port 3000
Server migrated to port 4000
Finally listening on port 5000
      `

      const port = await portService.detectPortFromLogContent(logContent)

      expect(port).toBe(5000)
    })
  })

  describe('detectPortFromLogs', () => {
    it('should read log file and detect port', async () => {
      const logContent = '➜  Local:   http://localhost:3000/'

      mockFs.readFile.mockResolvedValue(logContent)
      mockFs.access.mockResolvedValue(undefined) // File exists

      const port = await portService.detectPortFromLogs('/path/to/logs.txt')

      expect(port).toBe(3000)
      expect(mockFs.readFile).toHaveBeenCalledWith('/path/to/logs.txt', 'utf-8')
    })

    it('should return undefined when log file does not exist', async () => {
      mockFs.access.mockRejectedValue(new Error('File not found'))

      const port = await portService.detectPortFromLogs('/path/to/nonexistent.txt')

      expect(port).toBeUndefined()
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'Log file does not exist: /path/to/nonexistent.txt',
      )
    })

    it('should handle file read errors gracefully', async () => {
      mockFs.access.mockResolvedValue(undefined)
      mockFs.readFile.mockRejectedValue(new Error('Permission denied'))

      const port = await portService.detectPortFromLogs('/path/to/logs.txt')

      expect(port).toBeUndefined()
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'Error reading log file:',
        expect.objectContaining({
          logFile: '/path/to/logs.txt',
          error: 'Permission denied',
        }),
      )
    })
  })

  describe('detectAndValidatePort', () => {
    it('should detect and validate port successfully', async () => {
      const logContent = '➜  Local:   http://localhost:3000/'

      mockFs.access.mockResolvedValue(undefined)
      mockFs.readFile.mockResolvedValue(logContent)

      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: '1234\n',
      })

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const port = await portService.detectAndValidatePort('/path/to/logs.txt', 1234)

      expect(port).toBe(3000)
      expect(mockLogger.info).toHaveBeenCalledWith('Validated port 3000 belongs to PID 1234')
    })

    it('should return undefined when port does not belong to PID', async () => {
      const logContent = '➜  Local:   http://localhost:3000/'

      mockFs.access.mockResolvedValue(undefined)
      mockFs.readFile.mockResolvedValue(logContent)

      const mockExecAsync = vi.fn().mockResolvedValue({
        stdout: '5678\n',
      })

      vi.doMock('node:util', () => ({
        promisify: () => mockExecAsync,
      }))

      const port = await portService.detectAndValidatePort('/path/to/logs.txt', 1234)

      expect(port).toBeUndefined()
      expect(mockLogger.warn).toHaveBeenCalledWith(
        "Detected port 3000 but it doesn't belong to PID 1234",
      )
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
