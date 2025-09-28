import * as net from 'node:net'
import { createTaskLogger, type Logger } from '../utils/logger.js'

/**
 * Comprehensive port management service that handles all port-related operations
 * including detection, validation, PID checking, and cross-platform support
 */
export class PortService {
  private logger: Logger
  private static allocatedPorts = new Set<number>()

  constructor(logger?: Logger) {
    this.logger = logger || createTaskLogger('port-service')
  }

  // ===============================
  // Core Port Operations
  // ===============================

  /**
   * Finds the first available port starting from basePort
   * @param basePort - Starting port number (e.g., 9000 for preview, 6000 for proxy)
   * @param rangeSize - Number of ports to search (default: 1000)
   */
  async findAvailablePort(basePort: number, rangeSize = 1000): Promise<number> {
    const maxPort = basePort + rangeSize - 1

    for (let port = basePort; port <= maxPort; port++) {
      // Skip already allocated ports
      if (PortService.allocatedPorts.has(port)) {
        continue
      }

      if (await this.isPortAvailable(port)) {
        // Reserve the port immediately
        PortService.allocatedPorts.add(port)
        this.logger.debug(`Found and reserved available port ${port}`)
        return port
      }
    }

    throw new Error(`No available ports in range ${basePort}-${maxPort}`)
  }

  /**
   * Release a port back to the available pool
   */
  releasePort(port: number): void {
    PortService.allocatedPorts.delete(port)
    this.logger.debug(`Released port ${port}`)
  }

  /**
   * Finds any available port using the OS
   */
  async findAnyAvailablePort(): Promise<number> {
    return new Promise((resolve, reject) => {
      const server = net.createServer()
      server.listen(0, '127.0.0.1', () => {
        const address = server.address() as net.AddressInfo
        const port = address.port
        server.close(() => {
          this.logger.debug(`Found any available port: ${port}`)
          resolve(port)
        })
      })
      server.on('error', reject)
    })
  }

  /**
   * Try to connect to a host:port to check availability
   */
  private async tryConnect(
    host: string,
    port: number,
    timeoutMs = 200,
  ): Promise<'open' | 'refused' | 'other'> {
    return new Promise((resolve) => {
      const s = new net.Socket()

      const done = (result: 'open' | 'refused' | 'other') => {
        s.destroy()
        resolve(result)
      }

      s.setTimeout(timeoutMs, () => done('other'))

      s.once('connect', () => done('open'))
      s.once('error', (err: NodeJS.ErrnoException) => {
        if (err && (err.code === 'ECONNREFUSED' || err.code === 'EHOSTUNREACH')) {
          return done('refused')
        }
        done('other')
      })

      s.connect({ host, port })
    })
  }

  /**
   * Checks if a port is available (not in use)
   * Tests both IPv4 and IPv6 loopback addresses
   */
  async isPortAvailable(port: number, timeoutMs = 200): Promise<boolean> {
    // Check IPv4 and IPv6 loopback
    const results = await Promise.allSettled([
      this.tryConnect('127.0.0.1', port, timeoutMs),
      this.tryConnect('::1', port, timeoutMs),
    ])

    // If any path connects -> in use
    if (results.some((r) => r.status === 'fulfilled' && r.value === 'open')) {
      return false
    }

    // If both explicitly refused -> free
    if (results.every((r) => r.status === 'fulfilled' && r.value === 'refused')) {
      return true
    }

    // Ambiguous (timeout/other) -> treat as in use (conservative)
    return false
  }

  // ===============================
  // Server Connectivity
  // ===============================

  /**
   * Pings a port to check if it's responsive
   */
  async pingPort(port: number, timeout = 1000): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(timeout)

      client.on('connect', () => {
        client.destroy()
        resolve(true)
      })

      client.on('timeout', () => {
        client.destroy()
        resolve(false)
      })

      client.on('error', () => {
        resolve(false)
      })

      client.connect(port, '127.0.0.1')
    })
  }

  /**
   * Waits for a port to become available/responsive
   */
  async waitForPort(port: number, maxWait = 10000): Promise<boolean> {
    const checkInterval = 500
    let elapsed = 0

    this.logger.debug(`Waiting for port ${port} to become responsive...`)

    while (elapsed < maxWait) {
      if (await this.pingPort(port, 100)) {
        this.logger.debug(`Port ${port} is now responsive after ${elapsed}ms`)
        return true
      }

      await new Promise((resolve) => setTimeout(resolve, checkInterval))
      elapsed += checkInterval
    }

    this.logger.debug(`Port ${port} did not become responsive within ${maxWait}ms`)
    return false
  }
}

// Export a singleton instance for convenience
export const portService = new PortService()
