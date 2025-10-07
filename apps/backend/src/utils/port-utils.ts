import * as net from 'node:net'
import type { Logger } from './logger.js'

/**
 * Try to connect to a host:port to check availability
 */
function tryConnect(
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
export async function isPortAvailable(port: number, timeoutMs = 200): Promise<boolean> {
  // Check IPv4 and IPv6 loopback
  const results = await Promise.allSettled([
    tryConnect('127.0.0.1', port, timeoutMs),
    tryConnect('::1', port, timeoutMs),
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

/**
 * Finds an available port in the ephemeral port range (49152-65535)
 * Uses random selection to minimize conflicts when called in parallel
 */
export async function getPort(): Promise<number> {
  const MIN_PORT = 49152 // IANA ephemeral port range start
  const MAX_PORT = 65535 // IANA ephemeral port range end
  const maxAttempts = 10

  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    // Pick random port in ephemeral range
    const port = Math.floor(Math.random() * (MAX_PORT - MIN_PORT + 1)) + MIN_PORT

    if (await isPortAvailable(port)) {
      return port
    }
  }

  throw new Error(`Could not find available port after ${maxAttempts} attempts`)
}

/**
 * Finds any available port using the OS
 */
export function findAnyAvailablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address() as net.AddressInfo
      const port = address.port
      server.close(() => {
        resolve(port)
      })
    })
    server.on('error', reject)
  })
}

/**
 * Pings a port to check if it's responsive
 */
export function pingPort(port: number, timeout = 1000): Promise<boolean> {
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
export async function waitForPort(
  port: number,
  maxWait = 10000,
  logger?: Logger,
): Promise<boolean> {
  const checkInterval = 500
  let elapsed = 0

  if (logger) {
    logger.debug(`Waiting for port ${port} to become responsive...`)
  }

  while (elapsed < maxWait) {
    if (await pingPort(port, 100)) {
      if (logger) {
        logger.debug(`Port ${port} is now responsive after ${elapsed}ms`)
      }
      return true
    }

    await new Promise((resolve) => setTimeout(resolve, checkInterval))
    elapsed += checkInterval
  }

  if (logger) {
    logger.debug(`Port ${port} did not become responsive within ${maxWait}ms`)
  }
  return false
}
