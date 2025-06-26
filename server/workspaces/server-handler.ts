import { type ChildProcess, exec, spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import type { ServerStatus } from '@shared/types/workspace.js'
import { createTaskLogger } from '../utils/logger.js'

const execAsync = promisify(exec)

interface ServerStartResult {
  running: boolean
  port: number
  pid: number
  log_file: string
  uptime_seconds: number
}

export class ServerHandler {
  private process?: ChildProcess
  private port?: number
  private pid?: number
  private logFile?: string
  private startTime?: number
  private originalRequestedPort?: number
  private logger = createTaskLogger('server-handler')

  constructor(
    private workspacePath: string,
    private serverCommand: string,
    private portPlaceholder = '{PORT}',
  ) {
    this.logger.info('ServerHandler initialized with:', {
      workspacePath,
      serverCommand,
      portPlaceholder,
    })
  }

  async start(logFile?: string): Promise<ServerStartResult> {
    if (this.process && this.process.exitCode === null) {
      throw new Error('Server is already running')
    }

    // Find available port
    this.port = await this.findFreePort()
    this.originalRequestedPort = this.port
    this.logger.info('Found available port:', { port: this.port })

    // Replace port placeholder in command
    const originalCommand = this.serverCommand
    const command = this.serverCommand.replace(this.portPlaceholder, String(this.port))

    this.logger.info('Port placeholder replacement:', {
      originalCommand,
      placeholder: this.portPlaceholder,
      port: this.port,
      finalCommand: command,
    })

    // Check if the command actually contains the placeholder
    if (!originalCommand.includes(this.portPlaceholder)) {
      this.logger.warn(`Port placeholder '${this.portPlaceholder}' not found in command!`)
      this.logger.warn('The server will likely start on its default port.')
    }

    // Setup log file
    if (logFile) {
      this.logFile = logFile
    } else {
      const tempDir = path.join(tmpdir(), 'swarm_server_logs')
      await fs.mkdir(tempDir, { recursive: true })
      this.logFile = path.join(tempDir, `server_${Date.now()}_${this.port}.log`)
    }

    this.logger.info(`Starting server on port ${this.port} with command: ${command}`)
    this.logger.info(`Server logs will be saved to: ${this.logFile}`)

    // Create write stream for logs
    const logStream = await fs.open(this.logFile, 'a')

    try {
      // Start the process
      this.process = spawn(command, [], {
        cwd: this.workspacePath,
        shell: true,
        detached: process.platform !== 'win32',
        stdio: ['ignore', logStream.fd, logStream.fd],
      })

      this.pid = this.process.pid
      this.startTime = Date.now()

      // Wait for server to start
      const maxWait = 10 // seconds
      const checkInterval = 0.5
      let elapsed = 0

      this.logger.debug(`Waiting up to ${maxWait} seconds for server to start...`)

      // Track pre-existing ports
      const preExistingPorts = new Set<number>()
      const commonPorts = [3000, 3001, 3002, 8000, 8080, 4000, 5000]
      for (const port of commonPorts) {
        if (await this.checkPortInUse(port)) {
          preExistingPorts.add(port)
          this.logger.debug(`Port ${port} was already in use before starting server`)
        }
      }

      while (elapsed < maxWait) {
        await new Promise((resolve) => setTimeout(resolve, checkInterval * 1000))
        elapsed += checkInterval

        // Check if process is still running
        if (this.process.exitCode !== null) {
          break
        }

        // Check if port is now in use
        if (await this.checkPortInUse(this.port)) {
          this.logger.info(`Server is now listening on port ${this.port}`)
          break
        }

        // Try to detect actual port from logs
        const actualPort = await this.detectPortFromLogs()
        if (actualPort) {
          this.logger.info(`Detected server running on port ${actualPort} from logs`)
          this.port = actualPort
          break
        }

        // Give server minimum time to start
        if (elapsed < 2) {
        }
      }

      // Check final status
      const portInUse = await this.checkPortInUse(this.port)
      this.logger.info(`Final port check - Port ${this.port} in use: ${portInUse}`)

      // Try one more time to detect from logs
      if (!portInUse) {
        const actualPort = await this.detectPortFromLogs()
        if (actualPort && actualPort !== this.port) {
          this.logger.info(`Final detection: server running on port ${actualPort}`)
          this.port = actualPort
          // Check if this detected port is actually in use
          const actualPortInUse = await this.checkPortInUse(actualPort)
          this.logger.info(`Detected port ${actualPort} in use: ${actualPortInUse}`)
        }
      }

      if (this.process.exitCode !== null) {
        await logStream.close()

        // Check if server is running despite process exit
        if (!portInUse) {
          let errorLogs = ''
          try {
            const content = await fs.readFile(this.logFile, 'utf-8')
            errorLogs = content.slice(-1000) // Last 1000 chars
          } catch {
            // Ignore read errors
          }
          throw new Error(
            `Server failed to start. Exit code: ${this.process.exitCode}. Check logs at ${this.logFile}. Recent logs: ${errorLogs}`,
          )
        } else {
          this.logger.info(`Parent process exited but server is running on port ${this.port}`)
        }
      }

      await logStream.close()

      this.logger.info(`Server started successfully on port ${this.port} with PID ${this.pid}`)

      if (this.port !== this.originalRequestedPort) {
        this.logger.info(
          `Note: Server is using port ${this.port} instead of requested port ${this.originalRequestedPort}`,
        )
      }

      return {
        running: true,
        port: this.port,
        pid: this.pid!,
        log_file: this.logFile,
        uptime_seconds: 0,
      }
    } catch (error) {
      await logStream.close()
      this.cleanup()
      throw new Error(`Failed to start server: ${error}`)
    }
  }

  async stop(timeout = 10): Promise<boolean> {
    if (!this.process || this.process.exitCode !== null) {
      this.logger.warn('Server is not running')
      return false
    }

    this.logger.info(`Stopping server with PID ${this.pid}`)

    try {
      // Try graceful shutdown first
      if (process.platform !== 'win32') {
        // On Unix, send SIGTERM to process group
        process.kill(-this.pid!, 'SIGTERM')
      } else {
        // On Windows, just kill the process
        this.process.kill('SIGTERM')
      }

      // Wait for graceful shutdown
      const shutdownPromise = new Promise<void>((resolve) => {
        this.process!.on('exit', () => resolve())
      })

      const timeoutPromise = new Promise<void>((_, reject) => {
        setTimeout(() => reject(new Error('Timeout')), timeout * 1000)
      })

      try {
        await Promise.race([shutdownPromise, timeoutPromise])
        this.logger.info('Server stopped gracefully')
      } catch {
        // Force kill if graceful shutdown failed
        this.logger.warn('Graceful shutdown timed out, force killing...')
        if (process.platform !== 'win32') {
          process.kill(-this.pid!, 'SIGKILL')
        } else {
          this.process.kill('SIGKILL')
        }
        await shutdownPromise
        this.logger.info('Server force killed')
      }

      this.cleanup()
      return true
    } catch (error) {
      this.logger.error('Error stopping server:', error as Error)
      this.cleanup()
      return false
    }
  }

  getStatus(configured = true): ServerStatus {
    // Check if process is running
    if (!this.process) {
      this.logger.debug('No process object')
      // Even without process, check if port is in use
      if (this.port && this.checkPortInUseSync(this.port)) {
        this.logger.debug(`No process object but port ${this.port} is in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `localhost:${this.port}`,
          pid: this.pid,
          uptimeSeconds: uptime,
        }
      }
      return {
        running: false,
        configured,
        uptimeSeconds: 0,
      }
    }

    if (this.process.exitCode !== null) {
      this.logger.debug(`Process exited with code: ${this.process.exitCode}`)
      // Check if port is still in use
      if (this.port && this.checkPortInUseSync(this.port)) {
        this.logger.debug(`Process exited but port ${this.port} is still in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `localhost:${this.port}`,
          pid: this.pid,
          uptimeSeconds: uptime,
        }
      }
      this.cleanup()
      return {
        running: false,
        configured,
        uptimeSeconds: 0,
      }
    }

    // Process is running
    const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
    this.logger.debug(`Process ${this.pid} is running on port ${this.port}`)
    return {
      running: true,
      configured,
      url: `localhost:${this.port}`,
      pid: this.pid,
      uptimeSeconds: uptime,
    }
  }

  async getLogs(lines = 100): Promise<string> {
    if (!this.logFile || !(await this.pathExists(this.logFile))) {
      return 'No log file available'
    }

    try {
      if (process.platform !== 'win32') {
        // Use tail on Unix
        const { stdout } = await execAsync(`tail -n ${lines} "${this.logFile}"`)
        return stdout
      } else {
        // On Windows, read the whole file
        const content = await fs.readFile(this.logFile, 'utf-8')
        const allLines = content.split('\n')
        return allLines.slice(-lines).join('\n')
      }
    } catch (error) {
      this.logger.error('Error reading logs:', error as Error)
      return `Error reading logs: ${error}`
    }
  }

  private async findFreePort(): Promise<number> {
    return new Promise((resolve, reject) => {
      const server = net.createServer()
      server.listen(0, '127.0.0.1', () => {
        const address = server.address() as net.AddressInfo
        const port = address.port
        server.close(() => resolve(port))
      })
      server.on('error', reject)
    })
  }

  private async checkPortInUse(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(100)

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

  private checkPortInUseSync(port: number): boolean {
    // Since we need a synchronous check, we'll use the child_process execSync
    try {
      const { execSync } = require('node:child_process')
      if (process.platform === 'win32') {
        // Windows: Use netstat to check if port is in use
        try {
          execSync(`netstat -an | findstr :${port}`, { stdio: 'ignore' })
          return true
        } catch {
          return false
        }
      } else {
        // Unix/Linux/Mac: Use lsof to check if port is in use
        try {
          execSync(`lsof -i :${port}`, { stdio: 'ignore' })
          return true
        } catch {
          return false
        }
      }
    } catch {
      // If commands fail, fall back to assuming port is not in use
      return false
    }
  }

  private async detectPortFromLogs(): Promise<number | undefined> {
    if (!this.logFile || !(await this.pathExists(this.logFile))) {
      return undefined
    }

    try {
      const content = await fs.readFile(this.logFile, 'utf-8')

      // Common patterns for port detection
      const patterns = [
        /http:\/\/localhost:(\d+)/gi,
        /Local:\s+http:\/\/localhost:(\d+)/gi,
        /listening on port (\d+)/gi,
        /Server running on port (\d+)/gi,
        /Started server on .*:(\d+)/gi,
        /Listening on .*:(\d+)/gi,
      ]

      for (const pattern of patterns) {
        const matches = [...content.matchAll(pattern)]
        if (matches.length > 0) {
          // Return the last match (most recent)
          const port = parseInt(matches[matches.length - 1][1], 10)
          this.logger.debug(`Detected port ${port} from logs using pattern: ${pattern}`)
          return port
        }
      }
    } catch (error) {
      this.logger.debug('Error detecting port from logs:', error as Error)
    }

    return undefined
  }

  private cleanup(): void {
    this.process = undefined
    this.pid = undefined
    this.port = undefined
    this.startTime = undefined
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}
