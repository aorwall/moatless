import { describe, expect, it, vi } from 'vitest'
import { findAnyAvailablePort, findAvailablePort, PORT_RANGES } from '../port-allocation.js'

// Mock node:net
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

describe('Port Allocation Utils', () => {
  describe('PORT_RANGES', () => {
    it('should have correct port ranges', () => {
      expect(PORT_RANGES.PREVIEW).toEqual({ min: 3000, max: 3999 })
      expect(PORT_RANGES.WORKER).toEqual({ min: 4000, max: 4999 })
      expect(PORT_RANGES.GENERAL).toEqual({ min: 8000, max: 8999 })
    })
  })

  describe('findAvailablePort', () => {
    it('should find available port in preview range', async () => {
      const port = await findAvailablePort('preview')
      expect(port).toBeGreaterThanOrEqual(3000)
      expect(port).toBeLessThanOrEqual(3999)
    })

    it('should find available port in worker range', async () => {
      const port = await findAvailablePort('worker')
      expect(port).toBeGreaterThanOrEqual(4000)
      expect(port).toBeLessThanOrEqual(4999)
    })

    it('should find available port in general range by default', async () => {
      const port = await findAvailablePort()
      expect(port).toBeGreaterThanOrEqual(8000)
      expect(port).toBeLessThanOrEqual(8999)
    })

    it('should find available port in general range when specified', async () => {
      const port = await findAvailablePort('general')
      expect(port).toBeGreaterThanOrEqual(8000)
      expect(port).toBeLessThanOrEqual(8999)
    })

    it('should throw error for invalid port type', async () => {
      // @ts-expect-error - testing invalid input
      await expect(findAvailablePort('invalid')).rejects.toThrow('Invalid port type: invalid')
    })
  })

  describe('findAnyAvailablePort', () => {
    it('should find any available port using OS', async () => {
      const port = await findAnyAvailablePort()
      expect(port).toBe(50000) // Mocked port
    })
  })
})
