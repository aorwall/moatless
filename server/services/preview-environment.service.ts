import { execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import path from 'node:path'
import { TaskNotFoundError, WorkspaceError } from '@shared/constants/errors.js'
import type {
  PreviewEnvironmentMetadata,
  PreviewEnvironmentOptions,
  ProcessValidation,
} from '@shared/types/preview-environment.js'
import type { ServerStatus } from '@shared/types/workspace.js'
import type { FastifyInstance } from 'fastify'
import { logger } from '../utils/logger.js'
import { ServerHandler } from '../workspaces/server-handler.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

interface FrameworkDetection {
  type: 'vite' | 'nextjs' | 'cra' | 'webpack' | 'unknown'
  version?: string
  configFiles: string[]
}

interface ScriptInjectionStrategy {
  method: 'html-injection' | 'middleware' | 'build-plugin' | 'proxy'
  config?: {
    indexPath?: string
    insertLocation?: string
    middleware?: string
  }
}

export class PreviewEnvironmentService {
  private runningServers: Map<string, ServerHandler> = new Map()
  private readonly previewEnvironmentsPath: string

  constructor(
    private storage: StorageBackend,
    private workspaceService: WorkspaceService,
  ) {
    // Initialize preview environments storage path
    this.previewEnvironmentsPath = path.join(process.cwd(), 'data', 'preview-environments')
    this.initializeStorage()
  }

  private async initializeStorage(): Promise<void> {
    try {
      await fs.mkdir(this.previewEnvironmentsPath, { recursive: true })
      // Load and validate existing preview environment metadata on startup
      await this.loadAndValidateExistingPreviewEnvironments()
    } catch (error) {
      logger.error('Failed to initialize preview environment storage', error as Error)
    }
  }

  /**
   * Load existing preview environment metadata and validate processes on service startup
   */
  private async loadAndValidateExistingPreviewEnvironments(): Promise<void> {
    try {
      const files = await fs.readdir(this.previewEnvironmentsPath)
      const jsonFiles = files.filter((f) => f.endsWith('.json'))

      for (const file of jsonFiles) {
        const taskId = file.replace('.json', '')
        try {
          const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
          if (metadata) {
            // Validate if the process is still running
            const validation = await this.validateProcess(metadata.pid, metadata.command)

            if (validation.exists && validation.isCorrectProcess) {
              logger.info('Recovered running preview environment server', {
                taskId,
                pid: metadata.pid,
                port: metadata.port,
              })

              // Create a ServerHandler instance for the recovered server
              const handler = new ServerHandler(
                metadata.workspacePath,
                metadata.command,
                metadata.portPlaceholder,
              )

              // Restore the handler's state
              this.restoreServerHandler(handler, metadata)
              this.runningServers.set(taskId, handler)
            } else {
              logger.info('Cleaning up orphaned preview environment metadata', {
                taskId,
                pid: metadata.pid,
                validation,
              })
              await this.deletePreviewEnvironmentMetadata(taskId)
            }
          }
        } catch (error) {
          logger.error(`Failed to validate preview environment for task ${taskId}`, error as Error)
        }
      }
    } catch (error) {
      logger.error('Failed to load existing preview environments', error as Error)
    }
  }

  /**
   * Start a preview environment server for a task
   */
  async startServer(taskId: string): Promise<ServerStatus> {
    const previewLogger = logger.child({ taskId })

    // Validate task exists
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if server is already running
    const existingServer = this.runningServers.get(taskId)
    if (existingServer) {
      const status = existingServer.getStatus()
      if (status.running) {
        previewLogger.info('Server already running', { port: status.url })
        return status
      }
    }

    // Get workspace to access environment configuration
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const environment = workspace.environmentConfig

    if (!environment.codeExecution.serverCommand) {
      throw new WorkspaceError('No server command configured for this environment', taskId)
    }

    // Create server handler
    const workspacePath = workspace.getWorkspacePath()
    const handler = new ServerHandler(
      workspacePath,
      environment.codeExecution.serverCommand,
      environment.codeExecution.serverPortPlaceholder,
    )

    previewLogger.info('Starting preview environment server', {
      command: environment.codeExecution.serverCommand,
      workspacePath,
    })

    try {
      // Setup script injection before starting the server
      await this.setupScriptInjection(taskId, workspacePath)

      // Start the server
      const result = await handler.start()

      // Store the handler
      this.runningServers.set(taskId, handler)

      // Save metadata to disk
      const metadata: PreviewEnvironmentMetadata = {
        taskId,
        pid: result.pid,
        port: result.port,
        startTime: new Date(),
        command: environment.codeExecution.serverCommand,
        workspacePath,
        logFile: result.log_file,
        portPlaceholder: environment.codeExecution.serverPortPlaceholder,
        processStartTime: Date.now(),
      }

      await this.savePreviewEnvironmentMetadata(taskId, metadata)

      previewLogger.info('Preview environment server started successfully', {
        port: result.port,
        pid: result.pid,
      })

      return {
        running: true,
        configured: true,
        url: `http://localhost:${result.port}`,
        pid: result.pid,
        uptimeSeconds: 0,
      }
    } catch (error) {
      previewLogger.error('Failed to start preview environment server', error as Error)
      throw error
    }
  }

  /**
   * Stop a preview environment server
   */
  async stopServer(taskId: string, options?: PreviewEnvironmentOptions): Promise<boolean> {
    const previewLogger = logger.child({ taskId })

    const handler = this.runningServers.get(taskId)
    if (!handler) {
      previewLogger.debug('No server handler found')
      // Check if we have metadata for a potentially orphaned process
      const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
      if (metadata) {
        previewLogger.info('Found orphaned server metadata, attempting cleanup', {
          pid: metadata.pid,
        })
        await this.killOrphanedProcess(metadata.pid, options?.forceKill)
        await this.deletePreviewEnvironmentMetadata(taskId)
        return true
      }
      return false
    }

    try {
      const stopped = await handler.stop(options?.timeout)
      if (stopped) {
        this.runningServers.delete(taskId)
        await this.deletePreviewEnvironmentMetadata(taskId)

        // Cleanup injected scripts
        const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
        await this.cleanupScriptInjection(taskId, workspace.getWorkspacePath())

        previewLogger.info('Preview environment server stopped successfully')
      }
      return stopped
    } catch (error) {
      previewLogger.error('Error stopping preview environment server', error as Error)
      // Force cleanup
      this.runningServers.delete(taskId)
      await this.deletePreviewEnvironmentMetadata(taskId)

      try {
        // Attempt cleanup even if stop failed
        const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
        await this.cleanupScriptInjection(taskId, workspace.getWorkspacePath())
      } catch (cleanupError) {
        previewLogger.error('Failed to cleanup during error handling', cleanupError as Error)
      }

      return false
    }
  }

  /**
   * Restart a preview environment server
   */
  async restartServer(taskId: string): Promise<ServerStatus> {
    const previewLogger = logger.child({ taskId, operation: 'restartServer' })
    previewLogger.info('Restarting preview environment server')

    try {
      // First stop the server if it's running
      const stopped = await this.stopServer(taskId)
      previewLogger.debug(`Server stopped: ${stopped}`)

      // Wait a moment for the server to fully stop
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Then start it again
      const result = await this.startServer(taskId)
      previewLogger.info('Preview environment server restarted successfully')

      return result
    } catch (error) {
      previewLogger.error('Error restarting preview environment server', error as Error)
      throw error
    }
  }

  /**
   * Get server status for a task
   */
  async getServerStatus(taskId: string): Promise<ServerStatus> {
    // Validate task exists
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Check if we can manage server for this task
    const canManage = await this.workspaceService.canManageServer(taskId)
    if (!canManage) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }

    // Get workspace to check configuration
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const configured = Boolean(workspace.environmentConfig.codeExecution.serverCommand)

    // Check running servers
    const handler = this.runningServers.get(taskId)
    if (handler) {
      const status = handler.getStatus(configured)

      // Perform ping check if server is running
      if (status.running && status.url) {
        const port = this.extractPortFromUrl(status.url)
        const responsive = port ? await this.pingServer(port) : false

        return {
          ...status,
          responsive,
        } as ServerStatus
      }

      return status
    }

    // Check for orphaned server via metadata
    const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
    if (metadata) {
      const validation = await this.validateProcess(metadata.pid, metadata.command)
      if (validation.exists && validation.isCorrectProcess) {
        // Process is still running, recover it
        const handler = new ServerHandler(
          metadata.workspacePath,
          metadata.command,
          metadata.portPlaceholder,
        )

        this.restoreServerHandler(handler, metadata)
        this.runningServers.set(taskId, handler)

        const responsive = await this.pingServer(metadata.port)
        const uptime = Math.floor((Date.now() - metadata.startTime.getTime()) / 1000)

        return {
          running: true,
          configured,
          url: `http://localhost:${metadata.port}`,
          pid: metadata.pid,
          uptimeSeconds: uptime,
          responsive,
        } as ServerStatus
      } else {
        // Clean up invalid metadata
        await this.deletePreviewEnvironmentMetadata(taskId)
      }
    }

    return {
      running: false,
      configured,
      uptimeSeconds: 0,
    }
  }

  /**
   * Get server logs
   */
  async getServerLogs(taskId: string, lines = 100): Promise<string> {
    const handler = this.runningServers.get(taskId)
    if (!handler) {
      // Try to get logs from metadata
      const metadata = await this.loadPreviewEnvironmentMetadata(taskId)
      if (metadata?.logFile) {
        try {
          const content = await fs.readFile(metadata.logFile, 'utf-8')
          const allLines = content.split('\n')
          return allLines.slice(-lines).join('\n')
        } catch (_error) {
          return 'Failed to read log file'
        }
      }
      return 'No server logs available'
    }

    return await handler.getLogs(lines)
  }

  /**
   * Validate if a process is still running and is the correct one
   */
  private async validateProcess(pid: number, expectedCommand: string): Promise<ProcessValidation> {
    try {
      if (process.platform === 'win32') {
        // Windows: Use wmic to get process info
        try {
          const result = execSync(
            `wmic process where ProcessId=${pid} get CommandLine,CreationDate /format:list`,
            { encoding: 'utf-8' },
          )

          const lines = result.split('\n').filter((line) => line.trim())
          const commandLine = lines.find((line) => line.startsWith('CommandLine='))

          if (commandLine) {
            const command = commandLine.split('=')[1]
            return {
              exists: true,
              isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
              command,
            }
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      } else {
        // Unix/Linux/Mac: Use ps to get process info
        try {
          const result = execSync(`ps -p ${pid} -o command=`, { encoding: 'utf-8' })
          const command = result.trim()

          return {
            exists: true,
            isCorrectProcess: command.includes(expectedCommand.split(' ')[0]),
            command,
          }
        } catch {
          return { exists: false, isCorrectProcess: false }
        }
      }
    } catch (error) {
      logger.debug('Error validating process', { pid, error })
      return { exists: false, isCorrectProcess: false }
    }

    return { exists: false, isCorrectProcess: false }
  }

  /**
   * Extract port number from URL string
   */
  private extractPortFromUrl(url?: string): number | null {
    if (!url) return null

    try {
      // Handle both formats: "http://localhost:port" and "localhost:port"
      const parts = url.split(':')

      if (parts.length >= 2) {
        // If URL starts with http://, port is at index 2
        // If URL is just localhost:port, port is at index 1
        const portString = url.startsWith('http://') ? parts[2] : parts[parts.length - 1]
        const port = parseInt(portString, 10)

        // Validate port range
        if (Number.isNaN(port) || port < 0 || port > 65535) {
          return null
        }

        return port
      }
    } catch (error) {
      logger.debug('Error extracting port from URL', { url, error })
    }

    return null
  }

  /**
   * Ping a server to check if it's responsive
   */
  private async pingServer(port: number, timeout = 1000): Promise<boolean> {
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
   * Kill an orphaned process
   */
  private async killOrphanedProcess(pid: number, force = false): Promise<void> {
    try {
      if (process.platform === 'win32') {
        execSync(`taskkill ${force ? '/F' : ''} /PID ${pid}`)
      } else {
        process.kill(pid, force ? 'SIGKILL' : 'SIGTERM')
      }
    } catch (error) {
      logger.debug('Failed to kill orphaned process', { pid, error })
    }
  }

  /**
   * Restore ServerHandler state from metadata
   */
  private restoreServerHandler(handler: ServerHandler, metadata: PreviewEnvironmentMetadata): void {
    // Use private property access to restore state
    // This is a bit hacky but necessary for recovery
    ;(handler as unknown as { port: number }).port = metadata.port
    ;(handler as unknown as { pid: number }).pid = metadata.pid
    ;(handler as unknown as { logFile: string }).logFile = metadata.logFile
    ;(handler as unknown as { startTime: number }).startTime = metadata.startTime.getTime()

    // Create a mock process object with event emitter functionality
    const mockProcess = {
      exitCode: null,
      on: (_event: string, _callback: (...args: unknown[]) => void) => {},
      removeAllListeners: (_event?: string) => {},
      kill: (_signal?: string) => {},
    }
    ;(handler as unknown as { process: typeof mockProcess }).process = mockProcess
  }

  /**
   * Save preview environment metadata to disk
   */
  private async savePreviewEnvironmentMetadata(
    taskId: string,
    metadata: PreviewEnvironmentMetadata,
  ): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    await fs.writeFile(filePath, JSON.stringify(metadata, null, 2), 'utf-8')
  }

  /**
   * Load preview environment metadata from disk
   */
  private async loadPreviewEnvironmentMetadata(
    taskId: string,
  ): Promise<PreviewEnvironmentMetadata | null> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      const content = await fs.readFile(filePath, 'utf-8')
      const data = JSON.parse(content)
      // Convert date strings back to Date objects
      data.startTime = new Date(data.startTime)
      return data
    } catch {
      return null
    }
  }

  /**
   * Delete preview environment metadata from disk
   */
  private async deletePreviewEnvironmentMetadata(taskId: string): Promise<void> {
    const filePath = path.join(this.previewEnvironmentsPath, `${taskId}.json`)
    try {
      await fs.unlink(filePath)
    } catch {
      // Ignore errors if file doesn't exist
    }
  }

  /**
   * Stop all running servers (useful for shutdown)
   */
  async stopAllServers(): Promise<void> {
    const taskIds = Array.from(this.runningServers.keys())
    logger.info('Stopping all preview environment servers', { count: taskIds.length })

    // Stop servers (which includes cleanup)
    const stopPromises = taskIds.map((taskId) => this.stopServer(taskId))
    await Promise.allSettled(stopPromises)

    this.runningServers.clear()
  }

  /**
   * Graceful shutdown - stops all servers and cleans up metadata files
   */
  async gracefulShutdown(): Promise<void> {
    logger.info('Starting graceful shutdown of preview environment service')

    try {
      // Get all running servers AND check preview env directory for metadata files
      const runningTaskIds = Array.from(this.runningServers.keys())
      const metadataTaskIds = await this.getAllMetadataFiles()

      // Combine and deduplicate task IDs from both sources
      const allTaskIds = [...new Set([...runningTaskIds, ...metadataTaskIds])]

      logger.info('Preview environment shutdown status', {
        runningServers: runningTaskIds.length,
        metadataFiles: metadataTaskIds.length,
        totalEnvironments: allTaskIds.length,
      })

      if (allTaskIds.length > 0) {
        logger.info('Stopping all preview environments', { taskIds: allTaskIds })

        const stopPromises = allTaskIds.map(async (taskId) => {
          try {
            // Try graceful stop first with 5 second timeout
            const stopped = await this.stopServer(taskId, { timeout: 5000 })
            if (!stopped) {
              // Force stop if graceful failed
              logger.warn('Graceful stop failed, forcing shutdown', { taskId })
              await this.stopServer(taskId, { timeout: 2000, forceKill: true })
            }
            return { taskId, success: true }
          } catch (error) {
            logger.error('Failed to stop server during shutdown', { taskId, error })
            return { taskId, success: false }
          }
        })

        const results = await Promise.allSettled(stopPromises)
        const successful = results.filter((r) => r.status === 'fulfilled' && r.value.success).length

        logger.info('Preview environment shutdown complete', {
          attempted: allTaskIds.length,
          successful,
          failed: allTaskIds.length - successful,
        })
      }

      // Clear the running servers map
      this.runningServers.clear()

      logger.info('Preview environment service shutdown complete')
    } catch (error) {
      logger.error('Error during graceful shutdown', error as Error)
      throw error
    }
  }

  /**
   * Get all metadata files in the preview environments directory
   */
  private async getAllMetadataFiles(): Promise<string[]> {
    try {
      const files = await fs.readdir(this.previewEnvironmentsPath)
      return files.filter((f) => f.endsWith('.json')).map((f) => f.replace('.json', ''))
    } catch (error) {
      logger.debug('Failed to read preview environments directory', error as Error)
      return []
    }
  }

  /**
   * Detect the framework type and version from workspace
   */
  private async detectFramework(workspacePath: string): Promise<FrameworkDetection> {
    const packageJsonPath = path.join(workspacePath, 'package.json')
    const configFiles: string[] = []

    try {
      // Read package.json to detect dependencies
      const packageJson = JSON.parse(await fs.readFile(packageJsonPath, 'utf-8'))
      const deps = { ...packageJson.dependencies, ...packageJson.devDependencies }

      // Check for framework-specific config files
      const potentialConfigs = [
        'vite.config.js',
        'vite.config.ts',
        'vite.config.mjs',
        'next.config.js',
        'next.config.mjs',
        'next.config.ts',
        'webpack.config.js',
        'webpack.config.ts',
        'react-scripts',
      ]

      for (const config of potentialConfigs) {
        try {
          await fs.access(path.join(workspacePath, config))
          configFiles.push(config)
        } catch {
          // File doesn't exist, continue
        }
      }

      // Detect framework based on dependencies and config files
      if (deps.vite || configFiles.some((f) => f.startsWith('vite.'))) {
        return {
          type: 'vite',
          version: deps.vite,
          configFiles: configFiles.filter((f) => f.startsWith('vite.')),
        }
      }

      if (deps.next || configFiles.some((f) => f.startsWith('next.'))) {
        return {
          type: 'nextjs',
          version: deps.next,
          configFiles: configFiles.filter((f) => f.startsWith('next.')),
        }
      }

      if (deps['react-scripts']) {
        return {
          type: 'cra',
          version: deps['react-scripts'],
          configFiles: [],
        }
      }

      if (deps.webpack || configFiles.some((f) => f.startsWith('webpack.'))) {
        return {
          type: 'webpack',
          version: deps.webpack,
          configFiles: configFiles.filter((f) => f.startsWith('webpack.')),
        }
      }
    } catch (error) {
      logger.debug('Failed to detect framework', { workspacePath, error })
    }

    return { type: 'unknown', configFiles }
  }

  /**
   * Get the preview inspector script content
   */
  private getPreviewInspectorScript(): string {
    return `
      /* Preview Inspector Script - Injected by Preview Environment Service */
      (function() {
        const CONFIG = {
          HIGHLIGHT_COLOR: '#0da2e7',
          HIGHLIGHT_BG: '#0da2e71a',
          DEBOUNCE_MS: 10,
          TOOLTIP_OFFSET: 25,
          Z: 10000,
          MAX_TOOLTIP_WIDTH: 200,
          SCROLL_DEBOUNCE_MS: 420,
          ALLOWED_ORIGINS: ['*'], // Allow all origins for iframe communication
          FULL_WIDTH_DELTA: 12,
        };

                 const ATTR_SELECTED = 'data-inspector-selected';
         const ATTR_HOVERED = 'data-inspector-hovered';

                 const sendToParent = (payload) => {
           try {
             // Always ensure source is set for all messages
             const messageWithSource = { ...payload, source: 'preview-iframe' };
             console.log('[Preview Inspector] Sending message to parent:', messageWithSource);
             window.parent.postMessage(messageWithSource, '*');
           } catch (err) {
             console.error('preview-inspector: could not post to parent', err);
           }
         };

        const debounce = (fn, wait) => {
          let handle;
          return (...args) => {
            clearTimeout(handle);
            handle = setTimeout(() => fn(...args), wait);
          };
        };

        function hookConsole() {
          ['log', 'warn', 'error', 'info', 'debug'].forEach((level) => {
            const original = console[level];
            console[level] = (...args) => {
              original.apply(console, args);
              const stack = level === 'error' || level === 'warn'
                ? new Error().stack?.split('\\n').slice(2).join('\\n')
                : undefined;
              sendToParent({
                type: 'console',
                level,
                message: args.map((a) => {
                  if (typeof a === 'object') {
                    try { return JSON.stringify(a, null, 2); }
                    catch (e) { return String(a); }
                  }
                  return String(a);
                }).join(' '),
                args,
                stack,
                logged_at: new Date().toISOString(),
              });
            };
          });
        }

        function hookRuntimeErrors() {
          window.addEventListener('error', (ev) => {
            sendToParent({
              type: 'console',
              level: 'error',
              message: ev.message + ' at ' + ev.filename + ':' + ev.lineno + ':' + ev.colno,
              stack: ev.error?.stack,
            });
          });
          window.addEventListener('unhandledrejection', (ev) => {
            sendToParent({
              type: 'console',
              level: 'error',
              message: 'Unhandled Promise Rejection: ' + String(ev.reason),
              stack: ev.reason?.stack,
            });
          });
        }

        function hookFetch() {
          const original = window.fetch;
          window.fetch = async (...args) => {
            const started = performance.now();
            try {
              const resp = await original(...args);
              sendToParent({
                type: 'NETWORK_REQUEST',
                payload: {
                  url: args[0] ?? resp.url,
                  method: args[1]?.method ?? 'GET',
                  status: resp.status,
                  statusText: resp.statusText,
                  duration: performance.now() - started,
                  timestamp: new Date().toISOString(),
                  origin: location.origin,
                },
              });
              return resp;
            } catch (err) {
              sendToParent({
                type: 'NETWORK_REQUEST',
                payload: {
                  url: args[0],
                  method: args[1]?.method ?? 'GET',
                  error: { message: err.message, stack: err.stack },
                  duration: performance.now() - started,
                  timestamp: new Date().toISOString(),
                  origin: location.origin,
                },
              });
              throw err;
            }
          };
        }

        function hookScroll() {
          const throttled = debounce(() => {
            sendToParent({
              type: 'SCROLL_POSITION',
              payload: {
                scrollY: window.scrollY,
                scrollHeight: document.documentElement.scrollHeight,
                clientHeight: window.innerHeight,
                timestamp: Date.now(),
              },
            });
          }, CONFIG.DEBOUNCE_MS);
          window.addEventListener('scroll', throttled, { passive: true });
        }

                 function buildOverlay() {
           const tooltip = Object.assign(document.createElement('div'), {
             className: 'inspector-tooltip',
            style: \`position:fixed;z-index:\${CONFIG.Z};pointer-events:none;background:\${CONFIG.HIGHLIGHT_COLOR};color:#fff;padding:4px 8px;border-radius:4px;font-weight:bold;white-space:nowrap;display:none;max-width:\${CONFIG.MAX_TOOLTIP_WIDTH}px;font-size:14px;box-shadow:0 2px 4px rgba(0,0,0,.2)\`,
          });
          document.body.appendChild(tooltip);

          const style = document.createElement('style');
          style.textContent = \`[\${ATTR_HOVERED}]::before,[\${ATTR_SELECTED}]::before{content:'';position:absolute;inset:0;outline:1px dashed \${CONFIG.HIGHLIGHT_COLOR};background:\${CONFIG.HIGHLIGHT_BG};pointer-events:none;z-index:\${CONFIG.Z}}[\${ATTR_SELECTED}]::before{outline-offset:3px}\`;
          document.head.appendChild(style);

          let enabled = false;
          let hovered;

          const showTooltip = (el) => {
            const { top, left, width } = el.getBoundingClientRect();
            tooltip.textContent = el.tagName.toLowerCase();
            tooltip.style.display = 'block';
            tooltip.style.opacity = '1';
            tooltip.style.left = \`\${left}px\`;
            tooltip.style.top = \`\${Math.max(0, top - CONFIG.TOOLTIP_OFFSET)}px\`;
            if (Math.abs(width - window.innerWidth) < 5) {
              tooltip.style.left = \`\${CONFIG.FULL_WIDTH_DELTA}px\`;
              tooltip.style.top = \`\${CONFIG.FULL_WIDTH_DELTA}px\`;
            }
          };

          const hideTooltip = () => (tooltip.style.display = 'none');

          const over = debounce((ev) => {
            if (!enabled) return;
            const target = ev.target;
            if (!(target instanceof HTMLElement)) return;
            hovered?.removeAttribute(ATTR_HOVERED);
            hovered = target;
            showTooltip(target);
            target.setAttribute(ATTR_HOVERED, 'true');
          }, CONFIG.DEBOUNCE_MS);

          const out = debounce(() => {
            hovered?.removeAttribute(ATTR_HOVERED);
            hideTooltip();
          }, CONFIG.DEBOUNCE_MS);

          const click = (ev) => {
            if (!enabled) return;
            ev.preventDefault();
            ev.stopPropagation();
            const target = ev.target;
            if (!(target instanceof HTMLElement)) return;
            target.setAttribute(ATTR_SELECTED, 'true');
            const attrs = [...target.attributes].reduce((acc, a) => ({ ...acc, [a.name]: a.value }), {});
            sendToParent({
              type: 'ELEMENT_CLICKED',
              payload: { tagName: target.tagName.toLowerCase(), attrs, text: target.innerText, rect: target.getBoundingClientRect() },
              isMultiSelect: ev.metaKey || ev.ctrlKey,
            });
          };

          const toggle = (state) => {
            enabled = state;
            tooltip.style.display = 'none';
            if (enabled) {
              document.addEventListener('mouseover', over);
              document.addEventListener('mouseout', out);
              document.addEventListener('click', click, true);
              document.body.style.cursor = 'crosshair';
            } else {
              document.removeEventListener('mouseover', over);
              document.removeEventListener('mouseout', out);
              document.removeEventListener('click', click, true);
              document.body.style.cursor = '';
              document.querySelectorAll(\`[\${ATTR_HOVERED}], [\${ATTR_SELECTED}]\`).forEach((el) =>
                [ATTR_HOVERED, ATTR_SELECTED].forEach((a) => el.removeAttribute(a))
              );
            }
          };

          return { toggle };
        }

                // Wait for DOM to be ready before bootstrapping
        function waitForDOM() {
          return new Promise((resolve) => {
            if (document.readyState === 'loading') {
              document.addEventListener('DOMContentLoaded', resolve, { once: true });
            } else {
              resolve();
            }
          });
        }

        // Bootstrap everything
        (async function() {
          console.log('[Preview Inspector] Script starting to execute...');
          
          // Wait for DOM to be ready
          await waitForDOM();
          console.log('[Preview Inspector] DOM is ready');
          
          let selectorEnabled = false; // Track selector state globally
          const overlay = buildOverlay();
          console.log('[Preview Inspector] Overlay built');

          window.addEventListener('message', (ev) => {
            console.log('[Preview Inspector] Received message from parent:', ev.data);
            const { type, payload } = ev.data ?? {};
            switch (type) {
              case 'TOGGLE_SELECTOR':
                console.log('[Preview Inspector] Toggling selector:', payload);
                selectorEnabled = Boolean(payload);
                overlay.toggle(selectorEnabled);
                break;
              case 'SELECTOR_STATE_REQUEST':
                sendToParent({ type: 'SELECTOR_STATE_RESPONSE', payload: { isActive: selectorEnabled } });
                break;
            }
          });

          console.log('[Preview Inspector] Setting up hooks...');
          hookConsole();
          console.log('[Preview Inspector] Console hooks installed');
          
          hookRuntimeErrors();
          console.log('[Preview Inspector] Error hooks installed');
          
          hookFetch();
          console.log('[Preview Inspector] Fetch hooks installed');
          
          hookScroll();
          console.log('[Preview Inspector] Scroll hooks installed');
          
          console.log('[Preview Inspector] Sending SCRIPT_LOADED message...');
          sendToParent({ type: 'INSPECTOR_SCRIPT_LOADED', payload: { version: '2.0.0-server' } });
          
          console.log('[Preview Inspector] Bootstrap complete!');
          
          // Test console hooking - this should appear in the parent console
          setTimeout(() => {
            console.log('[Preview Inspector] Test message - console hooking is working!');
            console.warn('[Preview Inspector] Test warning - this should also appear in parent');
            console.error('[Preview Inspector] Test error - and this too');
          }, 1000);
        })().catch(err => {
          console.error('[Preview Inspector] Bootstrap failed:', err);
        });
    `
  }

  /**
   * Determine script injection strategy based on framework
   */
  private getScriptInjectionStrategy(framework: FrameworkDetection): ScriptInjectionStrategy {
    switch (framework.type) {
      case 'vite':
        return {
          method: 'html-injection', // Vite serves index.html that we can modify
          config: {
            indexPath: 'index.html',
            insertLocation: 'head',
          },
        }

      case 'nextjs':
        return {
          method: 'middleware', // Next.js can use middleware to inject scripts
          config: {
            middleware: 'api',
          },
        }

      case 'cra':
        return {
          method: 'html-injection', // CRA uses public/index.html
          config: {
            indexPath: 'public/index.html',
            insertLocation: 'head',
          },
        }

      default:
        return {
          method: 'html-injection', // Generic HTML injection as fallback
          config: {
            insertLocation: 'head',
          },
        }
    }
  }

  /**
   * Setup script injection for a workspace
   */
  private async setupScriptInjection(taskId: string, workspacePath: string): Promise<void> {
    const previewLogger = logger.child({ taskId, operation: 'setupScriptInjection' })

    try {
      const framework = await this.detectFramework(workspacePath)
      const strategy = this.getScriptInjectionStrategy(framework)

      previewLogger.info('Setting up script injection', {
        framework: framework.type,
        strategy: strategy.method,
        configFiles: framework.configFiles,
      })

      switch (strategy.method) {
        case 'html-injection':
          await this.injectIntoHtml(workspacePath, strategy.config)
          break
        case 'middleware':
          await this.setupMiddleware(workspacePath, framework, strategy.config)
          break
        default:
          previewLogger.warn('Unsupported injection method, falling back to HTML injection')
          await this.injectIntoHtml(workspacePath, {})
      }
    } catch (error) {
      previewLogger.error('Failed to setup script injection', error as Error)
      // Don't throw - let the server start without injection if needed
    }
  }

  /**
   * Inject script into HTML files
   */
  private async injectIntoHtml(
    workspacePath: string,
    config: ScriptInjectionStrategy['config'],
  ): Promise<void> {
    const possibleHtmlFiles = [
      config?.indexPath,
      'index.html',
      'public/index.html',
      'dist/index.html',
      'build/index.html',
    ].filter((file): file is string => Boolean(file))

    const script = this.getPreviewInspectorScript()
    const scriptTag = `<script type="text/javascript">${script}</script>`

    for (const htmlPath of possibleHtmlFiles) {
      const fullPath = path.join(workspacePath, htmlPath)

      try {
        let html = await fs.readFile(fullPath, 'utf-8')

        // Check if script is already injected to avoid duplicates
        if (html.includes('Preview Inspector Script - Injected by Preview Environment Service')) {
          continue
        }

        // Inject before closing head tag or at the beginning of body
        if (html.includes('</head>')) {
          html = html.replace('</head>', `  ${scriptTag}\n</head>`)
        } else if (html.includes('<body')) {
          html = html.replace(/<body[^>]*>/, `$&\n  ${scriptTag}`)
        } else {
          // Fallback: add at the beginning of the document
          html = `${scriptTag}\n${html}`
        }

        await fs.writeFile(fullPath, html, 'utf-8')
        logger.info('Successfully injected preview inspector script', { file: fullPath })
        return // Success - exit after first successful injection
      } catch (_error) {}
    }

    logger.warn('Could not find suitable HTML file for script injection', { workspacePath })
  }

  /**
   * Setup middleware for framework-specific injection
   */
  private async setupMiddleware(
    workspacePath: string,
    framework: FrameworkDetection,
    _config: ScriptInjectionStrategy['config'],
  ): Promise<void> {
    // For Next.js and other frameworks that support middleware
    if (framework.type === 'nextjs') {
      await this.setupNextjsMiddleware(workspacePath)
    }
    // Add other framework-specific middleware setup here
  }

  /**
   * Setup Next.js middleware for script injection
   */
  private async setupNextjsMiddleware(workspacePath: string): Promise<void> {
    const middlewarePath = path.join(workspacePath, 'middleware.js')
    // Note: For Next.js, the script injection would happen via middleware
    // Currently using static middleware creation instead of inline script injection

    const middlewareCode = `
import { NextResponse } from 'next/server'

export function middleware(request) {
  const response = NextResponse.next()
  
  // Only inject script for HTML responses
  if (request.nextUrl.pathname === '/' || request.nextUrl.pathname.endsWith('.html')) {
    response.headers.set('x-inspector-inject', 'true')
  }
  
  return response
}

export const config = {
  matcher: ['/((?!api|_next/static|_next/image|favicon.ico).*)'],
}
`

    try {
      // Check if middleware already exists
      try {
        const existing = await fs.readFile(middlewarePath, 'utf-8')
        if (existing.includes('x-inspector-inject')) {
          return // Already setup
        }
      } catch {
        // File doesn't exist, create it
      }

      await fs.writeFile(middlewarePath, middlewareCode, 'utf-8')
      logger.info('Created Next.js middleware for script injection', { middlewarePath })
    } catch (error) {
      logger.error('Failed to setup Next.js middleware', error as Error)
    }
  }

  /**
   * Cleanup injected scripts from workspace
   */
  private async cleanupScriptInjection(taskId: string, workspacePath: string): Promise<void> {
    const previewLogger = logger.child({ taskId, operation: 'cleanupScriptInjection' })

    try {
      const framework = await this.detectFramework(workspacePath)

      previewLogger.info('Cleaning up script injection', {
        framework: framework.type,
        workspacePath,
      })

      // Clean up HTML files
      await this.cleanupHtmlInjection(workspacePath)

      // Clean up framework-specific files
      if (framework.type === 'nextjs') {
        await this.cleanupNextjsMiddleware(workspacePath)
      }
    } catch (error) {
      previewLogger.error('Failed to cleanup script injection', error as Error)
      // Don't throw - continue with server stop even if cleanup fails
    }
  }

  /**
   * Remove injected scripts from HTML files
   */
  private async cleanupHtmlInjection(workspacePath: string): Promise<void> {
    const possibleHtmlFiles = [
      'index.html',
      'public/index.html',
      'dist/index.html',
      'build/index.html',
    ]

    for (const htmlPath of possibleHtmlFiles) {
      const fullPath = path.join(workspacePath, htmlPath)

      try {
        const html = await fs.readFile(fullPath, 'utf-8')

        // Check if script was injected
        if (!html.includes('Preview Inspector Script - Injected by Preview Environment Service')) {
          continue
        }

        // Remove the injected script tag
        const scriptPattern =
          /<script type="text\/javascript">[\s\S]*?\/\* Preview Inspector Script - Injected by Preview Environment Service \*\/[\s\S]*?<\/script>\s*/g
        const cleanedHtml = html.replace(scriptPattern, '')

        if (cleanedHtml !== html) {
          await fs.writeFile(fullPath, cleanedHtml, 'utf-8')
          logger.info('Cleaned up preview inspector script', { file: fullPath })
        }
      } catch (_error) {}
    }
  }

  /**
   * Cleanup Next.js middleware if it was created by us
   */
  private async cleanupNextjsMiddleware(workspacePath: string): Promise<void> {
    const middlewarePath = path.join(workspacePath, 'middleware.js')

    try {
      const existing = await fs.readFile(middlewarePath, 'utf-8')

      // Only remove if it contains our marker and nothing else important
      if (existing.includes('x-inspector-inject') && existing.includes('NextResponse')) {
        // This is likely our generated middleware, remove it
        await fs.unlink(middlewarePath)
        logger.info('Cleaned up Next.js middleware', { middlewarePath })
      }
    } catch (_error) {
      // File doesn't exist or can't be removed, ignore
    }
  }
}

export function createPreviewEnvironmentService(
  _app: FastifyInstance,
  storage: StorageBackend,
  workspaceService: WorkspaceService,
): PreviewEnvironmentService {
  return new PreviewEnvironmentService(storage, workspaceService)
}
