import { type ChildProcess, spawn } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { ServerStatus } from '@moatless/types'
import { execAsync } from '../utils/git-utils.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { PortService } from './port.service.js'

interface ServerStartResult {
  running: boolean
  port: number
  pid: number
  log_file: string
  uptime_seconds: number
  starting?: boolean
}

export class PreviewServer {
  private process?: ChildProcess
  private port?: number
  private actualPort?: number
  private pid?: number
  private logFile?: string
  private startTime?: number
  private failureReason?: string
  private logger = createTaskLogger('preview-server')
  private portService = new PortService(this.logger)
  private readonly portPlaceholder = '{PORT}'

  constructor(
    private taskId: string,
    private workspacePath: string,
    private serverCommand: string,
    private envVars: Record<string, string> = {},
  ) {
    this.logger.info('PreviewServer initialized with:', {
      taskId: this.taskId,
      workspacePath,
      serverCommand,
      portPlaceholder: this.portPlaceholder,
      envVars: Object.keys(envVars),
    })
  }

  async start(logFile?: string): Promise<ServerStartResult> {
    if (this.process && this.process.exitCode === null) {
      throw new Error('Server is already running')
    }

    // Clean up any previous state
    this.cleanup()

    const originalCommand = this.serverCommand
    let command = this.serverCommand
    let allocatedPort: number | undefined

    // Only allocate a port if the command contains the placeholder
    if (originalCommand.includes(this.portPlaceholder)) {
      allocatedPort = await findAvailablePort('preview')
      this.port = allocatedPort
      command = this.serverCommand.replace(this.portPlaceholder, String(this.port))

      this.logger.info('Port placeholder replacement:', {
        originalCommand,
        placeholder: this.portPlaceholder,
        allocatedPort: this.port,
        finalCommand: command,
      })
    } else {
      this.logger.info('No port placeholder found - will detect actual port from server startup:', {
        originalCommand,
        placeholder: this.portPlaceholder,
      })
    }

    // Setup log file
    if (logFile) {
      this.logFile = logFile
    } else {
      const tempDir = path.join(tmpdir(), 'swarm_server_logs')
      await fs.mkdir(tempDir, { recursive: true })
      this.logFile = path.join(tempDir, `server_${Date.now()}.log`)
    }

    this.logger.info(`Starting server with command: ${command}`)
    this.logger.info(`Server logs will be saved to: ${this.logFile}`)

    // Create write stream for logs
    const logStream = await fs.open(this.logFile, 'a')

    try {
      // Merge environment variables with current process environment
      const processEnv = {
        ...process.env,
        ...this.envVars,
      }

      // Start the process
      this.process = spawn(command, [], {
        cwd: this.workspacePath,
        shell: true,
        detached: process.platform !== 'win32',
        stdio: ['ignore', logStream.fd, logStream.fd],
        env: processEnv,
      })

      this.pid = this.process.pid
      this.startTime = Date.now()

      // Wait for server to start
      const maxWait = 10 // seconds
      const checkInterval = 0.5
      let elapsed = 0

      this.logger.debug(`Waiting up to ${maxWait} seconds for server to start...`)

      // For commands with port placeholder, check the allocated port
      // For commands without placeholder, scan for newly opened ports
      while (elapsed < maxWait) {
        await new Promise((resolve) => setTimeout(resolve, checkInterval * 1000))
        elapsed += checkInterval

        // Check if process is still running
        if (this.process.exitCode !== null) {
          break
        }

        if (this.port) {
          // We have an allocated port, check if it's in use
          if (await this.portService.isPortInUse(this.port)) {
            this.logger.info(`Server is now listening on allocated port ${this.port}`)
            break
          }
        }

        // Always try to detect actual port from logs with PID validation
        if (this.logFile && this.pid) {
          const actualPort = await this.portService.detectAndValidatePort(this.logFile, this.pid)
          if (actualPort) {
            this.logger.info(
              `Detected and validated server running on port ${actualPort} from logs`,
            )
            this.actualPort = actualPort
            this.port = actualPort
            break
          }
        }

        // Give server minimum time to start
        if (elapsed < 2) {
        }
      }

      // Final port detection
      if (!this.port && this.logFile && this.pid) {
        // Last attempt to detect from logs with PID validation
        const actualPort = await this.portService.detectAndValidatePort(this.logFile, this.pid)
        if (actualPort) {
          this.logger.info(`Final detection: validated server running on port ${actualPort}`)
          this.actualPort = actualPort
          this.port = actualPort
        }
      }

      // Verify the final port is actually in use
      const portInUse = this.port ? await this.portService.isPortInUse(this.port) : false
      this.logger.info(`Final port check - Port ${this.port || 'unknown'} in use: ${portInUse}`)

      if (this.process.exitCode !== null) {
        await logStream.close()

        // Check if server is running despite process exit
        if (!portInUse || !this.port) {
          let errorLogs = ''
          try {
            const content = await fs.readFile(this.logFile, 'utf-8')
            errorLogs = content.slice(-1000) // Last 1000 chars
          } catch {
            // Ignore read errors
          }
          // Extract useful error info but keep it clean
          this.failureReason = `Server process exited with code ${this.process.exitCode}${this.port ? '' : ' (no port detected)'}`
          throw new Error(
            `Server failed to start. Exit code: ${this.process.exitCode}. Port detected: ${this.port || 'none'}. Check logs at ${this.logFile}. Recent logs: ${errorLogs}`,
          )
        }
        this.logger.info(`Parent process exited but server is running on port ${this.port}`)
      }

      await logStream.close()

      // If we have a port, server is fully started
      if (this.port) {
        this.logger.info(`Server started successfully on port ${this.port} with PID ${this.pid}`)
        return {
          running: true,
          port: this.port,
          pid: this.pid!,
          log_file: this.logFile,
          uptime_seconds: 0,
        }
      }

      // No port detected but process is still running - this is a failure
      if (this.process.exitCode === null) {
        this.logger.error(
          `Server process started with PID ${this.pid} but no port could be detected - killing process`,
        )

        // Kill the process since we can't determine its port
        try {
          if (process.platform !== 'win32') {
            process.kill(-this.pid!, 'SIGTERM')
          } else {
            this.process.kill('SIGTERM')
          }
        } catch (killError) {
          this.logger.error(
            'Error killing process after port detection failure',
            killError as Error,
          )
        }

        this.cleanup()
        throw new Error(`Failed to detect server port after startup. Check logs at ${this.logFile}`)
      }

      // Process exited and no port - complete failure
      this.failureReason = 'Server process exited and no port detected'
      throw new Error('Server process exited and no port detected')
    } catch (error) {
      await logStream.close()
      // Extract clean error message
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.failureReason = errorMessage.includes('Failed to start server')
        ? errorMessage.replace(/Check logs at.*$/, '').trim()
        : `Startup error: ${errorMessage}`
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
      if (this.port && this.portService.isPortInUseSync(this.port)) {
        this.logger.debug(`No process object but port ${this.port} is in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `http://localhost:${this.port}`,
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
      if (this.port && this.portService.isPortInUseSync(this.port)) {
        this.logger.debug(`Process exited but port ${this.port} is still in use`)
        const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
        return {
          running: true,
          configured,
          url: `http://localhost:${this.port}`,
          pid: this.pid,
          uptimeSeconds: uptime,
        }
      }
      // Don't cleanup here - preserve log access after failure
      return {
        running: false,
        configured,
        uptimeSeconds: 0,
        error: this.failureReason,
      }
    }

    // Process is running
    const uptime = this.startTime ? Math.floor((Date.now() - this.startTime) / 1000) : 0
    this.logger.debug(`Process ${this.pid} is running on port ${this.port}`)
    return {
      running: true,
      configured,
      url: `http://localhost:${this.port}`,
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
      }
      // On Windows, read the whole file
      const content = await fs.readFile(this.logFile, 'utf-8')
      const allLines = content.split('\n')
      return allLines.slice(-lines).join('\n')
    } catch (error) {
      this.logger.error('Error reading logs:', error as Error)
      return `Error reading logs: ${error}`
    }
  }

  // Port detection methods now handled by PortService

  private cleanup(): void {
    // Release port allocation
    if (this.port) {
      // Port tracking simplified - no need to release ports manually
    }

    this.process = undefined
    this.pid = undefined
    this.port = undefined
    this.actualPort = undefined
    this.startTime = undefined
    this.failureReason = undefined
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  getPort(): number | undefined {
    return this.actualPort || this.port
  }

  getActualPort(): number | undefined {
    return this.actualPort
  }

  async proxyRequest(request: {
    method: string
    path: string
    headers: Headers
    body?: string
  }): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    // Use actualPort if available, otherwise fall back to port
    const targetPort = this.actualPort || this.port

    if (!targetPort) {
      throw new Error('No port available for proxying - server may not be running')
    }

    // Check if we have a process or if the port is in use (server might be running independently)
    if (!this.portService.isPortInUseSync(targetPort)) {
      throw new Error(`Server is not running on port ${targetPort}`)
    }

    const url = `http://localhost:${targetPort}${request.path}`
    this.logger.debug('Proxying request', {
      url,
      method: request.method,
      targetPort,
      actualPort: this.actualPort,
      allocatedPort: this.port,
      processRunning: this.process && this.process.exitCode === null,
    })

    try {
      const fetchOptions: RequestInit = {
        method: request.method,
        headers: request.headers,
      }

      if (request.body && request.method !== 'GET' && request.method !== 'HEAD') {
        fetchOptions.body = request.body
      }

      const response = await fetch(url, fetchOptions)
      const responseHeaders = new Headers()

      // Copy response headers, filtering out hop-by-hop headers
      const hopByHopHeaders = [
        'connection',
        'keep-alive',
        'proxy-authenticate',
        'proxy-authorization',
        'te',
        'trailer',
        'transfer-encoding',
        'upgrade',
      ]

      response.headers.forEach((value, key) => {
        if (!hopByHopHeaders.includes(key.toLowerCase())) {
          responseHeaders.set(key, value)
        }
      })

      // Read response body
      const contentType = response.headers.get('content-type') || ''
      let body: string | Buffer

      if (
        contentType.includes('text/') ||
        contentType.includes('application/json') ||
        contentType.includes('application/javascript')
      ) {
        body = await response.text()
      } else {
        body = Buffer.from(await response.arrayBuffer())
      }

      return {
        status: response.status,
        headers: responseHeaders,
        body,
      }
    } catch (error) {
      this.logger.error('Error proxying request', error as Error)
      throw error
    }
  }
}
