import { exec, execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import { promisify } from 'node:util'
import type { Logger } from 'pino'
import { createTaskLogger } from '../utils/logger.js'

const execAsync = promisify(exec)

// Port range constants
export const PORT_RANGES = {
  PREVIEW: { min: 3000, max: 3999 },
  WORKER: { min: 4000, max: 4999 },
  GENERAL: { min: 8000, max: 8999 },
} as const

export type PortType = 'preview' | 'worker' | 'general'

/**
 * Comprehensive port management service that handles all port-related operations
 * including detection, validation, PID checking, and cross-platform support
 */
export class PortService {
  private logger: Logger

  constructor(logger?: Logger) {
    this.logger = logger || createTaskLogger('port-service')
  }

  // ===============================
  // Core Port Operations
  // ===============================

  /**
   * Finds the first available port in the specified range
   */
  async findAvailablePort(type: PortType = 'general'): Promise<number> {
    const range = PORT_RANGES[type.toUpperCase() as keyof typeof PORT_RANGES]

    if (!range) {
      throw new Error(`Invalid port type: ${type}`)
    }

    for (let port = range.min; port <= range.max; port++) {
      if (await this.isPortAvailable(port)) {
        this.logger.debug(`Found available port ${port} for type: ${type}`)
        return port
      }
    }

    throw new Error(`No available ports in range ${range.min}-${range.max} for type: ${type}`)
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
   * Checks if a port is available (not in use)
   */
  async isPortAvailable(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(100)

      client.on('connect', () => {
        client.destroy()
        resolve(false) // Port is in use
      })

      client.on('timeout', () => {
        client.destroy()
        resolve(true) // Port is available
      })

      client.on('error', () => {
        resolve(true) // Port is available
      })

      client.connect(port, '127.0.0.1')
    })
  }

  /**
   * Checks if a port is in use (opposite of isPortAvailable)
   */
  async isPortInUse(port: number): Promise<boolean> {
    return !(await this.isPortAvailable(port))
  }

  /**
   * Synchronous check if a port is in use using system commands
   */
  isPortInUseSync(port: number): boolean {
    try {
      if (process.platform === 'win32') {
        // Windows: Use netstat to check if port is in use
        execSync(`netstat -an | findstr :${port}`, { stdio: 'ignore' })
        return true
      }
      // Unix/Linux/Mac: Use lsof to check if port is in use
      execSync(`lsof -i :${port}`, { stdio: 'ignore' })
      return true
    } catch {
      // Commands return error if no process found on port
      return false
    }
  }

  // ===============================
  // PID Validation Methods
  // ===============================

  /**
   * Gets all process IDs that are using a specific port
   */
  async getProcessesUsingPort(port: number): Promise<number[]> {
    try {
      if (process.platform === 'win32') {
        // Windows: Parse netstat -ano output
        const { stdout } = await execAsync(`netstat -ano | findstr :${port}`)
        return this.parseWindowsPids(stdout)
      }
      // Unix: Use lsof -t for clean PID list
      const { stdout } = await execAsync(`lsof -i :${port} -t`)
      const pids = stdout
        .trim()
        .split('\n')
        .map((p: string) => Number.parseInt(p, 10))
        .filter((p: number) => !isNaN(p))

      this.logger.debug(`Processes using port ${port}:`, { pids })
      return pids
    } catch (error) {
      this.logger.debug(`No processes found using port ${port}:`, {
        error: error instanceof Error ? error.message : String(error),
      })
      return []
    }
  }

  /**
   * Parses Windows netstat output to extract PIDs
   */
  private parseWindowsPids(netstatOutput: string): number[] {
    const pids: number[] = []
    const lines = netstatOutput.split('\n')

    for (const line of lines) {
      // Windows netstat format: TCP    0.0.0.0:PORT    0.0.0.0:0    LISTENING    PID
      const match = line.trim().match(/\s+(\d+)\s*$/)
      if (match) {
        const pid = Number.parseInt(match[1], 10)
        if (!isNaN(pid) && !pids.includes(pid)) {
          pids.push(pid)
        }
      }
    }

    return pids
  }

  /**
   * Checks if a specific process (PID) is using a port
   */
  async isPortUsedByProcess(port: number, pid: number): Promise<boolean> {
    const pids = await this.getProcessesUsingPort(port)
    return pids.includes(pid)
  }

  /**
   * Validates that a port belongs to a specific process or its children
   */
  async validatePortBelongsToPid(port: number, pid: number): Promise<boolean> {
    const pids = await this.getProcessesUsingPort(port)

    this.logger.debug('Validating port ownership:', {
      port,
      expectedPid: pid,
      actualPids: pids,
    })

    // Check if the exact PID is using the port
    if (pids.includes(pid)) {
      this.logger.debug(`Port ${port} confirmed to belong to PID ${pid}`)
      return true
    }

    // Check if any descendant processes are using the port
    const hasDescendantProcess = await this.checkChildProcesses(pid, pids)
    if (hasDescendantProcess) {
      this.logger.info(`Port ${port} belongs to descendant process of PID ${pid}`)
      return true
    }

    this.logger.warn(`Port ${port} does not belong to PID ${pid} or its descendants`, {
      port,
      expectedPid: pid,
      actualPids: pids,
    })
    return false
  }

  /**
   * Checks if any of the PIDs using a port are descendants of our main process
   */
  private async checkChildProcesses(parentPid: number, portPids: number[]): Promise<boolean> {
    try {
      // Get all descendant PIDs of our parent process
      const descendants = await this.getAllDescendantPids(parentPid)

      // Check if any PID using the port is a descendant
      for (const portPid of portPids) {
        if (descendants.has(portPid)) {
          this.logger.debug(`Port PID ${portPid} is a descendant of ${parentPid}`)
          return true
        }
      }
    } catch (error) {
      this.logger.debug('Error checking child processes:', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    return false
  }

  /**
   * Recursively gets all descendant PIDs of a parent process
   */
  private async getAllDescendantPids(parentPid: number): Promise<Set<number>> {
    const descendants = new Set<number>()

    try {
      const directChildren = await this.getDirectChildPids(parentPid)

      if (directChildren.length > 0) {
        this.logger.debug(`PID ${parentPid} has direct children: ${directChildren.join(', ')}`)
      }

      for (const childPid of directChildren) {
        descendants.add(childPid)
        // Recursively get grandchildren
        const grandChildren = await this.getAllDescendantPids(childPid)
        grandChildren.forEach((pid) => descendants.add(pid))
      }

      if (descendants.size > 0) {
        this.logger.debug(
          `PID ${parentPid} total descendants: ${Array.from(descendants).join(', ')}`,
        )
      }
    } catch (error) {
      this.logger.debug(`Error getting descendants for PID ${parentPid}:`, {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    return descendants
  }

  /**
   * Gets direct child PIDs of a parent process
   */
  private async getDirectChildPids(parentPid: number): Promise<number[]> {
    try {
      if (process.platform === 'win32') {
        // Windows: Use wmic to get child processes
        const { stdout } = await execAsync(
          `wmic process where ParentProcessId=${parentPid} get ProcessId /value`,
        )
        const matches = stdout.matchAll(/ProcessId=(\d+)/g)
        return Array.from(matches)
          .map((match) => Number.parseInt(match[1], 10))
          .filter((pid) => !isNaN(pid))
      }
      // Unix: Use ps to get child processes
      const { stdout } = await execAsync(`ps -o pid= --ppid ${parentPid}`)
      return stdout
        .trim()
        .split('\n')
        .map((line) => Number.parseInt(line.trim(), 10))
        .filter((pid) => !isNaN(pid))
    } catch (error) {
      this.logger.debug(`Error getting direct children for PID ${parentPid}:`, {
        error: error instanceof Error ? error.message : String(error),
      })
      return []
    }
  }

  // ===============================
  // Port Detection from Logs
  // ===============================

  /**
   * Detects port from log file content
   */
  async detectPortFromLogs(logFile: string): Promise<number | undefined> {
    if (!(await this.pathExists(logFile))) {
      this.logger.debug(`Log file does not exist: ${logFile}`)
      return undefined
    }

    try {
      const content = await fs.readFile(logFile, 'utf-8')
      return this.detectPortFromLogContent(content)
    } catch (error) {
      this.logger.debug('Error reading log file:', {
        logFile,
        error: error instanceof Error ? error.message : String(error),
      })
      return undefined
    }
  }

  /**
   * Detects port from log content using common patterns
   */
  async detectPortFromLogContent(content: string): Promise<number | undefined> {
    // Strip ANSI color codes and escape sequences from log content
    // Construct regex dynamically to avoid control character warning
    const ansiEscapeSequence = String.fromCharCode(27) + '\\[[0-9;]*[mGKH]'
    const ansiRegex = new RegExp(ansiEscapeSequence, 'g')
    const cleanContent = content.replace(ansiRegex, '')

    // Common patterns for port detection - order matters, more specific first
    const patterns = [
      /➜\s+Local:\s+http:\/\/localhost:(\d+)/gi, // Vite specific
      /Local:\s+http:\/\/localhost:(\d+)/gi,
      /http:\/\/localhost:(\d+)/gi,
      /listening on port (\d+)/gi,
      /Server running on port (\d+)/gi,
      /Started server on .*:(\d+)/gi,
      /Listening on .*:(\d+)/gi,
      /server started.*:(\d+)/gi,
      /running on.*:(\d+)/gi,
      /available on.*:(\d+)/gi,
    ]

    this.logger.debug('Scanning logs for port patterns...', {
      logLength: content.length,
      cleanedLength: cleanContent.length,
    })

    for (const pattern of patterns) {
      const matches = [...cleanContent.matchAll(pattern)]
      if (matches.length > 0) {
        // Return the last match (most recent)
        const port = Number.parseInt(matches[matches.length - 1][1], 10)
        this.logger.info(`Detected port ${port} from logs using pattern: ${pattern}`)
        return port
      }
    }

    this.logger.debug('No port patterns matched in logs')
    return undefined
  }

  /**
   * Detects port from logs and validates it belongs to the specified PID
   */
  async detectAndValidatePort(logFile: string, pid: number): Promise<number | undefined> {
    const detectedPort = await this.detectPortFromLogs(logFile)

    if (detectedPort) {
      // Give the server a moment to fully start up before checking
      await new Promise((resolve) => setTimeout(resolve, 200))

      // First check if port is actually in use
      const portInUse = await this.isPortInUse(detectedPort)
      if (!portInUse) {
        // Try the synchronous check as backup
        const portInUseSync = this.isPortInUseSync(detectedPort)
        this.logger.debug(
          `Port ${detectedPort} check - async: ${portInUse}, sync: ${portInUseSync}`,
        )

        if (!portInUseSync) {
          this.logger.warn(
            `Detected port ${detectedPort} but it's not in use (both async and sync checks failed)`,
          )
          return undefined
        }
        this.logger.info(`Port ${detectedPort} detected by sync check despite async failure`)
      }

      // Try to validate the port belongs to our process or its descendants
      if (await this.validatePortBelongsToPid(detectedPort, pid)) {
        this.logger.info(`Validated port ${detectedPort} belongs to PID ${pid}`)
        return detectedPort
      }

      // If strict validation fails, but we detected the port from the correct log file
      // and the port is actually in use, we can be more lenient for development servers
      this.logger.info(
        `Port ${detectedPort} detected from process ${pid} logs and is in use - accepting despite PID validation failure`,
      )
      return detectedPort
    }

    return undefined
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

  // ===============================
  // Utility Methods
  // ===============================

  /**
   * Checks if a file path exists
   */
  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}

// Export a singleton instance for convenience
export const portService = new PortService()
