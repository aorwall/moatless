import { describe, expect, it, vi } from 'vitest'
import {
  findAnyAvailablePort,
  getPort,
  isPortAvailable,
  pingPort,
  waitForPort,
} from '../port-utils.js'

describe('port-utils', () => {
  describe('isPortAvailable', () => {
    it('should return true when port is available', async () => {
      // Use a very high port number that's likely available
      const port = 60_000 + Math.floor(Math.random() * 1000)
      const available = await isPortAvailable(port)
      expect(available).toBe(true)
    })

    it('should return false when port is in use', async () => {
      const net = await import('node:net')
      const server = net.createServer()

      // Start a server on a random port
      await new Promise<void>((resolve) => {
        server.listen(0, '127.0.0.1', () => resolve())
      })

      const address = server.address() as { port: number }
      const port = address.port

      try {
        const available = await isPortAvailable(port)
        expect(available).toBe(false)
      } finally {
        await new Promise<void>((resolve) => {
          server.close(() => resolve())
        })
      }
    })

    it('should handle custom timeout', async () => {
      const port = 60_000
      const available = await isPortAvailable(port, 100)
      expect(available).toBe(true)
    })
  })

  describe('getPort', () => {
    it('should return a port in the ephemeral range', async () => {
      const port = await getPort()
      expect(port).toBeGreaterThanOrEqual(49_152)
      expect(port).toBeLessThanOrEqual(65_535)
    })

    it('should return an available port', async () => {
      const port = await getPort()
      const available = await isPortAvailable(port)
      expect(available).toBe(true)
    })

    it('should return different ports on subsequent calls', async () => {
      const port1 = await getPort()
      const port2 = await getPort()
      // They might be the same due to randomness, but should both be valid
      expect(port1).toBeGreaterThanOrEqual(49_152)
      expect(port2).toBeGreaterThanOrEqual(49_152)
    })
  })

  describe('findAnyAvailablePort', () => {
    it('should return an OS-assigned port', async () => {
      const port = await findAnyAvailablePort()
      expect(port).toBeGreaterThan(0)
      expect(port).toBeLessThanOrEqual(65_535)
    })

    it('should return an available port', async () => {
      const port = await findAnyAvailablePort()
      const available = await isPortAvailable(port)
      expect(available).toBe(true)
    })
  })

  describe('pingPort', () => {
    it('should return false for a port with no server', async () => {
      const port = 60_000 + Math.floor(Math.random() * 1000)
      const responsive = await pingPort(port, 100)
      expect(responsive).toBe(false)
    })

    it('should return true for a running server', async () => {
      const net = await import('node:net')
      const server = net.createServer()

      await new Promise<void>((resolve) => {
        server.listen(0, '127.0.0.1', () => resolve())
      })

      const address = server.address() as { port: number }
      const port = address.port

      try {
        const responsive = await pingPort(port, 1000)
        expect(responsive).toBe(true)
      } finally {
        await new Promise<void>((resolve) => {
          server.close(() => resolve())
        })
      }
    })
  })

  describe('waitForPort', () => {
    it('should return false when port never becomes available', async () => {
      const port = 60_000 + Math.floor(Math.random() * 1000)
      const ready = await waitForPort(port, 500)
      expect(ready).toBe(false)
    })

    it('should return true when server starts', async () => {
      const net = await import('node:net')
      const server = net.createServer()

      const port = await findAnyAvailablePort()

      // Start server after a short delay
      const serverPromise = new Promise<void>((resolve) => {
        setTimeout(() => {
          server.listen(port, '127.0.0.1', () => resolve())
        }, 100)
      })

      try {
        const ready = await waitForPort(port, 2000)
        expect(ready).toBe(true)
        await serverPromise
      } finally {
        await new Promise<void>((resolve) => {
          server.close(() => resolve())
        })
      }
    })

    it('should call callbacks during wait', async () => {
      const port = 60_000 + Math.floor(Math.random() * 1000)
      const onWaiting = vi.fn()
      const onTimeout = vi.fn()

      await waitForPort(port, 500, {
        onWaiting,
        onTimeout,
      })

      expect(onWaiting).toHaveBeenCalledWith(port)
      expect(onTimeout).toHaveBeenCalledWith(port, 500)
    })

    it('should call onReady when port becomes available', async () => {
      const net = await import('node:net')
      const server = net.createServer()

      const port = await findAnyAvailablePort()

      const onReady = vi.fn()

      // Start server immediately
      await new Promise<void>((resolve) => {
        server.listen(port, '127.0.0.1', () => resolve())
      })

      try {
        await waitForPort(port, 2000, {
          onReady,
        })

        expect(onReady).toHaveBeenCalled()
        expect(onReady.mock.calls[0][0]).toBe(port)
        expect(onReady.mock.calls[0][1]).toBeGreaterThanOrEqual(0)
      } finally {
        await new Promise<void>((resolve) => {
          server.close(() => resolve())
        })
      }
    })
  })
})
