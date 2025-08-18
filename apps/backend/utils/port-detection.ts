import { execAsync } from './git-utils.js'
import { createLogger } from './logger.js'

const logger = createLogger()

/**
 * Get all listening TCP ports for a given PID and optionally its children
 * @param pid The process ID to check
 * @param includeChildren Whether to include ports from child processes
 * @returns Array of port numbers
 */
export async function getPortsForPid(pid: number, includeChildren = true): Promise<number[]> {
  try {
    // For macOS, use lsof which is more reliable
    if (process.platform === 'darwin') {
      return await getPortsForPidMacOS(pid, includeChildren)
    }
    // For Linux, use ss
    if (process.platform === 'linux') {
      return await getPortsForPidLinux(pid, includeChildren)
    }

    logger.warn(`Unsupported platform: ${process.platform}`)
    return []
  } catch (error) {
    logger.error('Failed to get ports for PID', { pid, error })
    return []
  }
}

async function getPortsForPidMacOS(pid: number, includeChildren: boolean): Promise<number[]> {
  const ports = new Set<number>()

  // Get ports for main process
  try {
    // Use lsof with -P to avoid port name resolution and -n to avoid hostname resolution
    // This ensures we only get ports that this specific PID is listening on
    const { stdout } = await execAsync(`lsof -Pan -p ${pid} -i TCP 2>/dev/null || true`)

    // Parse lsof output
    // Example line: "node    1234 user   23u  IPv6 0x123  0t0  TCP *:3000 (LISTEN)"
    const lines = stdout.split('\n')
    for (const line of lines) {
      // Only process lines that show LISTEN state
      if (!line.includes('(LISTEN)')) continue

      // Extract port from lines with various formats:
      // *:3000 (LISTEN)
      // localhost:3000 (LISTEN)
      // 127.0.0.1:3000 (LISTEN)
      const portMatch = line.match(/:(\d+)\s+\(LISTEN\)/)
      if (portMatch) {
        const port = Number.parseInt(portMatch[1], 10)
        if (port >= 1024 && port <= 65535) {
          ports.add(port)
          logger.debug(`Found listening port ${port} for PID ${pid}`)
        }
      }
    }
  } catch (error) {
    // Process might not exist or have no ports
    logger.debug(`Error getting ports for PID ${pid}: ${error}`)
  }

  // Get child processes if requested
  if (includeChildren) {
    try {
      // Get all child PIDs
      const { stdout } = await execAsync(`pgrep -P ${pid} 2>/dev/null || true`)
      const childPids = stdout
        .split('\n')
        .filter(Boolean)
        .map((p) => Number.parseInt(p, 10))

      logger.debug(`Found ${childPids.length} child processes for PID ${pid}`)

      // Recursively get ports for each child
      for (const childPid of childPids) {
        const childPorts = await getPortsForPidMacOS(childPid, true)
        for (const port of childPorts) {
          ports.add(port)
        }
      }
    } catch {
      // No children or error getting them
    }
  }

  return Array.from(ports).sort((a, b) => a - b)
}

async function getPortsForPidLinux(pid: number, includeChildren: boolean): Promise<number[]> {
  const ports = new Set<number>()

  try {
    // Use ss command for Linux
    const { stdout } = await execAsync(`ss -tulnp 2>/dev/null | grep "pid=${pid}" || true`)

    const lines = stdout.split('\n')
    for (const line of lines) {
      // Extract port from ss output
      // Example: "LISTEN 0 128 *:3000 *:* users:(("node",pid=1234,fd=23))"
      const portMatch = line.match(/:(\d+)\s/)
      if (portMatch) {
        const port = Number.parseInt(portMatch[1], 10)
        if (port >= 1024 && port <= 65535) {
          ports.add(port)
          logger.debug(`Found port ${port} for PID ${pid}`)
        }
      }
    }
  } catch (error) {
    // Process might not exist or have no ports
    logger.debug(`No ports found for PID ${pid}: ${error}`)
  }

  // Get child processes if requested
  if (includeChildren) {
    try {
      const { stdout } = await execAsync(`pgrep -P ${pid} 2>/dev/null || true`)
      const childPids = stdout
        .split('\n')
        .filter(Boolean)
        .map((p) => Number.parseInt(p, 10))

      logger.debug(`Found ${childPids.length} child processes for PID ${pid}`)

      for (const childPid of childPids) {
        const childPorts = await getPortsForPidLinux(childPid, true)
        for (const port of childPorts) {
          ports.add(port)
        }
      }
    } catch {
      // No children or error getting them
    }
  }

  return Array.from(ports).sort((a, b) => a - b)
}

/**
 * Find an HTTP/HTTPS port for a given PID
 * Tests each port to see if it responds to HTTP requests
 * @param pid The process ID to check
 * @returns The first HTTP port found, or null if none
 */
export async function findHttpPortForPid(pid: number): Promise<number | null> {
  const ports = await getPortsForPid(pid, true)

  if (ports.length === 0) {
    logger.debug(`No ports found for PID ${pid}`)
    return null
  }

  logger.debug(`Found ${ports.length} ports for PID ${pid}: ${ports.join(', ')}`)

  // Check each port to see if it responds to HTTP
  for (const port of ports) {
    try {
      // Try to connect with a very short timeout
      await fetch(`http://localhost:${port}`, {
        signal: AbortSignal.timeout(500),
        method: 'HEAD', // Use HEAD to avoid downloading content
      })

      // If we get any response (even error), it's likely an HTTP server
      logger.info(`Found HTTP server on port ${port} for PID ${pid}`)
      return port
    } catch (error) {
      // Check if it's an HTTP error response (still indicates HTTP server)
      if (error instanceof Error && error.message.includes('fetch')) {
        // Network error, not HTTP
        logger.debug(`Port ${port} is not HTTP or not ready yet`)
      } else {
        // Might be HTTP but returned error - still counts
        logger.info(`Found HTTP server on port ${port} for PID ${pid} (with error response)`)
        return port
      }
    }
  }

  // If no HTTP ports found but we have ports, return the first one
  // (it might be HTTP but not ready yet)
  if (ports.length > 0) {
    logger.info(`No confirmed HTTP ports for PID ${pid}, returning first port: ${ports[0]}`)
    return ports[0]
  }

  return null
}

/**
 * Wait for a process to start listening on an HTTP port
 * @param pid The process ID to monitor
 * @param timeout Maximum time to wait in milliseconds
 * @param checkInterval How often to check in milliseconds
 * @returns The port number when found, or null if timeout
 */
export async function waitForHttpPort(
  pid: number,
  timeout = 30000,
  checkInterval = 1000,
): Promise<number | null> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeout) {
    const port = await findHttpPortForPid(pid)
    if (port) {
      return port
    }

    // Wait before checking again
    await new Promise((resolve) => setTimeout(resolve, checkInterval))
  }

  logger.warn(`Timeout waiting for HTTP port for PID ${pid}`)
  return null
}
