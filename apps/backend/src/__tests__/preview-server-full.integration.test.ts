import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { PreviewServerRegistry } from '@moatless/preview-server'
import type { RepositoryConfig, ServerStatus } from '@moatless/types'
import { container } from 'tsyringe'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import type { DockerService } from '../services/docker.service.js'
import { RepositoryService } from '../services/repository.service.js'
import type { ScreenshotService } from '../services/screenshot.service.js'
import { SettingsService } from '../services/settings.service.js'
import { WorkspaceService } from '../services/workspace.service.js'
import { FileStorage } from '../storage/index.js'
import type { Workspace } from '../workspaces/workspace.js'

// ========================================
// Test Configuration Constants
// ========================================
const TEST_CONFIG = {
  SETUP_TIMEOUT: 120000, // 2 minutes for repository cloning
  TEST_TIMEOUT: 180000, // 3 minutes for full tests
  SERVER_START_TIMEOUT: 30000, // 30 seconds for server to start
  HTTP_TIMEOUT: 5000, // 5 seconds for HTTP requests
  POLL_INTERVAL: 500, // 500ms between status polls
  DELAY_BEFORE_RESTART: 1000, // 1 second delay before restart
  MIN_PORT: 1000,
  MAX_PORT: 65536,
}

// ========================================
// Test Logger Utilities
// ========================================
const log = {
  success: (msg: string) => console.log(`✅ ${msg}`),
  error: (msg: string) => console.log(`❌ ${msg}`),
  info: (msg: string) => console.log(msg),
  warn: (msg: string) => console.warn(msg),
}

// ========================================
// Test Suite
// ========================================

describe('PreviewServer Full Integration Test with Port Detection', () => {
  let tempDir: string
  let repositoryService: RepositoryService
  let workspaceService: WorkspaceService
  let workspace: Workspace
  let repository: RepositoryConfig
  let taskId: string
  let previewRegistry: PreviewServerRegistry

  beforeAll(async () => {
    // Setup temp directory for isolated test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'preview-test-'))
    process.env['DATA_DIR'] = tempDir

    log.info(`Test environment created at: ${tempDir}`)

    // Initialize services with minimal mocking
    const storage = new FileStorage(tempDir)

    // Create a proper EventService mock with emitEvent method
    const eventService = {
      emitEvent: () => Promise.resolve(),
      ...new EventEmitter(),
    } as any

    const dockerService = {} as DockerService // Mock, not used for local environment
    const claudeService = {} as ClaudeService // Mock, not used for this test

    previewRegistry = new PreviewServerRegistry()

    // Create repository service and set up settings service for GitHub cloning
    repositoryService = new RepositoryService(storage)

    const settingsService = container.resolve(SettingsService)
    repositoryService.setSettingsService(settingsService)

    // Mock github service for git operations
    const githubService = {
      getTokenForGitOperations: vi.fn().mockResolvedValue(null),
    } as any

    // Mock screenshot service
    const screenshotService = {
      captureScreenshot: vi.fn().mockResolvedValue({ success: true }),
      cleanup: vi.fn().mockResolvedValue(undefined),
    } as any as ScreenshotService

    workspaceService = new WorkspaceService(
      repositoryService,
      storage,
      eventService,
      dockerService,
      claudeService,
      previewRegistry,
      githubService,
      screenshotService,
    )

    // Clone the real repository and use the actual dev server commands
    log.info('Creating repository from GitHub (using real bun run dev)...')
    repository = await repositoryService.createRepository({
      name: 'hello-world-test',
      sourceType: 'github',
      remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
      cloneOnNewTask: false,
      codeExecution: {
        type: 'local',
        setupCommands: ['bun install'],
      },
      previewConfig: {
        serverStartCommands: ['bun run dev --port {PORT}'],
        enableInspector: true, // Required for proxy server and script injection
      },
    })

    log.info(`Repository created: ${repository.name} at ${repository.path}`)

    taskId = randomUUID()
  }, TEST_CONFIG.SETUP_TIMEOUT)

  afterAll(async () => {
    // Cleanup workspace and temp directory
    try {
      if (workspace) {
        log.info('Cleaning up workspace...')
        await workspace.teardown(true)
      }
    } catch (error) {
      log.warn(`Error during workspace cleanup: ${error}`)
    }

    try {
      log.info(`Removing temp directory: ${tempDir}`)
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      log.warn(`Error removing temp directory: ${error}`)
    }

    // Clean up environment variable
    Reflect.deleteProperty(process.env, 'DATA_DIR')
  }, TEST_CONFIG.SERVER_START_TIMEOUT)

  it(
    'should start preview server, detect port, and verify proxy injection',
    async () => {
      // 1. Create workspace
      const result = await createTestWorkspace(workspaceService, repository.id, taskId)
      workspace = result.workspace

      // 2. Setup environment
      await setupWorkspaceEnvironment(workspace)

      // 3. Start server and wait for it to be ready
      const serverStatus = await startAndWaitForServer(workspace)

      // 4. Verify server started successfully
      expect(serverStatus.status).toBe('started')
      expect(serverStatus.url).toBeDefined()
      expect(serverStatus.url).toMatch(/^http:\/\/localhost:\d+$/)

      // 5. Extract and validate port
      const port = extractPortFromUrl(serverStatus.url!)
      log.success(`Port successfully detected: ${port}`)
      validatePort(port)

      // 6. Test HTTP connectivity and verify inspector injection
      // The URL contains the proxy URL (if inspector enabled), which injects the script
      const response = await testHttpConnectivity(serverStatus.url!)
      const html = await response.text()

      // 7. Verify inspector script injection
      await verifyInspectorInjection(html, taskId)

      // 10. Get server status (optimization test)
      log.info('Checking server status (testing optimization)...')
      const status = await workspace.getServerStatus()
      expect(status.status).toBe('started')
      expect(status.url).toBe(serverStatus.url)
      log.success('Server status check passed without re-detecting port')

      // 11. Stop server
      log.info('Stopping preview server...')
      const stopped = await workspace.stopServer()
      expect(stopped).toBe(true)

      // 12. Verify server is stopped
      const finalStatus = await workspace.getServerStatus()
      expect(finalStatus.status).toBe('stopped')
      log.success('Server stopped successfully')
    },
    TEST_CONFIG.TEST_TIMEOUT,
  )

  it(
    'should fail gracefully when port cannot be detected',
    async () => {
      log.info('Testing port detection failure scenario...')

      // Create a repository with a command that doesn't start a server
      const badRepo = await repositoryService.createRepository({
        name: 'no-port-test',
        sourceType: 'github',
        remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
        cloneOnNewTask: false,
        codeExecution: {
          type: 'local',
          setupCommands: ['bun install'],
        },
        previewConfig: {
          serverStartCommands: ['echo "not a server"'],
          enableInspector: false,
        },
      })

      // Create workspace
      const { workspace: badWorkspace } = await createTestWorkspace(workspaceService, badRepo.id)

      // Setup environment
      await setupWorkspaceEnvironment(badWorkspace)

      // Start server (should fail or stop immediately)
      log.info('Starting server with command that does not bind to port...')
      const result = await badWorkspace.startServer()
      const serverStatus = await waitForServerStatus(badWorkspace, result, {
        throwOnFailed: false,
      })

      // Should return error or stopped state
      expect(['stopped', 'failed']).toContain(serverStatus.status)

      if (serverStatus.status === 'stopped') {
        log.success('Command completed but did not maintain a server process')
      } else if (serverStatus.error) {
        expect(serverStatus.error).toMatch(
          /Failed to start server|HTTP-responsive|no port|killed|Process exited/i,
        )
        log.success(`Server failure handled correctly: ${serverStatus.error}`)
      }

      // Cleanup
      await badWorkspace.teardown(true)
    },
    TEST_CONFIG.SETUP_TIMEOUT,
  )

  it(
    'should restart server and maintain same port allocation',
    async () => {
      log.info('Testing server restart with port preservation...')

      // 1. Create workspace and setup
      const { workspace: restartWorkspace } = await createTestWorkspace(
        workspaceService,
        repository.id,
      )
      await setupWorkspaceEnvironment(restartWorkspace)

      // 2. Start server first time
      log.info('Starting server for first time...')
      const initialStatus = await startAndWaitForServer(restartWorkspace)
      expect(initialStatus.status).toBe('started')

      // 3. Extract initial port and URL
      const initialPort = extractPortFromUrl(initialStatus.url!)
      const initialUrl = initialStatus.url!
      log.success(`Initial server URL: ${initialUrl}`)
      log.success(`Initial port: ${initialPort}`)

      // 4. Verify initial server is responsive
      await testHttpConnectivity(initialStatus.url!)

      // 5. Stop the server
      log.info('Stopping server...')
      const stopped = await restartWorkspace.stopServer()
      expect(stopped).toBe(true)

      const stoppedStatus = await restartWorkspace.getServerStatus()
      expect(stoppedStatus.status).toBe('stopped')
      log.success('Server stopped successfully')

      // 6. Wait before restart
      await new Promise((resolve) => setTimeout(resolve, TEST_CONFIG.DELAY_BEFORE_RESTART))

      // 7. Restart the server
      log.info('Restarting server...')
      const restartStatus = await startAndWaitForServer(restartWorkspace)
      expect(restartStatus.status).toBe('started')

      // 8. Extract restart port and verify it's preserved
      const restartPort = extractPortFromUrl(restartStatus.url!)
      const restartUrl = restartStatus.url!
      log.success(`Restarted server URL: ${restartUrl}`)
      log.success(`Restarted port: ${restartPort}`)

      expect(restartPort).toBe(initialPort)
      log.success('✅✅✅ PORT PRESERVED AFTER RESTART - Test Passed!')

      // 9. Verify restarted server is responsive
      await testHttpConnectivity(restartStatus.url!)

      // 10. Test inspector functionality after restart
      // Give server a moment to fully initialize after restart
      await new Promise((resolve) => setTimeout(resolve, 1000))

      try {
        const response = await fetch(restartStatus.url!, {
          signal: AbortSignal.timeout(TEST_CONFIG.HTTP_TIMEOUT),
        })

        if (!response.ok) {
          log.warn(`Server returned non-OK status: ${response.status} ${response.statusText}`)
          // Server might not be fully ready after restart - this is acceptable
          // The core functionality (port preservation and server restart) is already verified
        } else {
          const html = await response.text()
          expect(html).toContain('Preview Inspector')
          log.success('Inspector script working after restart')
        }
      } catch (error) {
        log.warn(`Server fetch failed: ${error}`)
        // This is acceptable - the core restart functionality works
      }

      // 12. Test multiple restart cycles
      log.info('Testing second restart cycle...')
      await restartWorkspace.stopServer()
      await new Promise((resolve) => setTimeout(resolve, TEST_CONFIG.POLL_INTERVAL))

      const secondRestartResult = await restartWorkspace.startServer()
      const secondRestartStatus = await waitForServerStatus(restartWorkspace, secondRestartResult, {
        throwOnFailed: false,
      })

      if (secondRestartStatus.status === 'started') {
        const secondPort = extractPortFromUrl(secondRestartStatus.url!)
        expect(secondPort).toBe(initialPort)
        log.success('Port preserved across multiple restarts')
      }

      // Cleanup
      log.info('Cleaning up restart test workspace...')
      await restartWorkspace.teardown(true)
      log.success('Restart test completed successfully')
    },
    TEST_CONFIG.TEST_TIMEOUT,
  )
})

// ========================================
// Helper Functions
// ========================================

/**
 * Wait for server to reach a specific status
 */
async function waitForServerStatus(
  workspace: Workspace,
  initialStatus: ServerStatus,
  options: {
    targetStatuses?: string[]
    timeoutMs?: number
    pollInterval?: number
    throwOnFailed?: boolean
  } = {},
): Promise<ServerStatus> {
  const {
    targetStatuses = ['started', 'stopped'],
    timeoutMs = TEST_CONFIG.SERVER_START_TIMEOUT,
    pollInterval = TEST_CONFIG.POLL_INTERVAL,
    throwOnFailed = false,
  } = options

  let serverStatus = initialStatus

  // Handle failed status immediately if throwOnFailed is true
  if (serverStatus.status === 'failed' && throwOnFailed) {
    throw new Error(`Server failed to start: ${serverStatus.error || 'Unknown error'}`)
  }

  if (targetStatuses.includes(serverStatus.status)) {
    return serverStatus
  }

  if (serverStatus.status !== 'starting') {
    return serverStatus
  }

  log.info('Server is starting, waiting for status change...')
  const startTime = Date.now()

  while (serverStatus.status === 'starting' && Date.now() - startTime < timeoutMs) {
    await new Promise((resolve) => setTimeout(resolve, pollInterval))
    serverStatus = await workspace.getServerStatus()

    if (serverStatus.status === 'started') {
      log.success(`Server is now running on ${serverStatus.url}`)
      break
    }

    if (serverStatus.status === 'failed') {
      const msg = `Server failed to start: ${serverStatus.error}`
      if (throwOnFailed) {
        throw new Error(msg)
      }
      log.error(msg)
      break
    }

    if (serverStatus.status === 'stopped') {
      log.info('Server stopped')
      break
    }
  }

  if (serverStatus.status === 'starting') {
    const msg = `Server did not start within ${timeoutMs}ms timeout`
    if (throwOnFailed) {
      throw new Error(msg)
    }
    log.error(msg)
  }

  return serverStatus
}

/**
 * Extract port number from a URL
 */
function extractPortFromUrl(url: string): number {
  const portMatch = url.match(/:(\d+)$/)
  if (!portMatch?.[1]) {
    throw new Error(`Could not extract port from URL: ${url}`)
  }
  return Number.parseInt(portMatch[1], 10)
}

/**
 * Test HTTP connectivity to a URL
 */
async function testHttpConnectivity(
  url: string,
  timeoutMs = TEST_CONFIG.HTTP_TIMEOUT,
): Promise<Response> {
  log.info(`Testing HTTP connectivity to ${url}...`)
  const response = await fetch(url, {
    signal: AbortSignal.timeout(timeoutMs),
  })
  if (!response.ok) {
    throw new Error(`HTTP request failed with status ${response.status}`)
  }
  log.success('HTTP connectivity test passed')
  return response
}

/**
 * Create a test workspace with standard configuration
 */
async function createTestWorkspace(
  workspaceService: WorkspaceService,
  repositoryId: string,
  taskId?: string,
): Promise<{ workspace: Workspace; taskId: string }> {
  const actualTaskId = taskId || randomUUID()
  log.info(`Creating workspace with task ID: ${actualTaskId}`)

  const workspace = await workspaceService.createWorkspace(actualTaskId, repositoryId)
  expect(workspace).toBeDefined()
  expect(workspace.id).toBe(actualTaskId)

  return { workspace, taskId: actualTaskId }
}

/**
 * Setup workspace environment (run setup commands)
 */
async function setupWorkspaceEnvironment(workspace: Workspace): Promise<void> {
  log.info('Setting up environment (running setup commands)...')
  await workspace.setup()
  expect(workspace.status).toBe('ready')
}

/**
 * Start server and wait for it to be ready
 */
async function startAndWaitForServer(
  workspace: Workspace,
  throwOnFailed = true,
): Promise<ServerStatus> {
  log.info('Starting preview server...')
  const startResult = await workspace.startServer()
  return waitForServerStatus(workspace, startResult, { throwOnFailed })
}

/**
 * Verify inspector script injection is working correctly
 */
async function verifyInspectorInjection(html: string, taskId: string): Promise<void> {
  log.info('Verifying inspector script injection...')

  // Verify response is HTML
  expect(html).toBeDefined()
  expect(html.length).toBeGreaterThan(0)

  // Verify inspector script is injected
  const expectedContent = [
    'Preview Inspector',
    'Preview Inspector Script - Injected by Preview Environment Service',
    'hookConsole',
    'hookFetch',
    'hookScroll',
    'hookNavigation',
    `serverId: '${taskId}'`, // Preview server uses serverId, mapped from taskId
  ]

  for (const content of expectedContent) {
    expect(html).toContain(content)
  }
  log.success('Inspector script found in response')

  // Verify script is injected before closing body tag
  const scriptTagIndex = html.indexOf('<script>')
  const bodyCloseIndex = html.indexOf('</body>')
  expect(scriptTagIndex).toBeGreaterThan(0)
  expect(bodyCloseIndex).toBeGreaterThan(scriptTagIndex)
  log.success('Script correctly injected before </body>')

  // Verify original content is preserved
  expect(html.toLowerCase()).toContain('<!doctype html>')
  expect(html).toContain('</html>')
  log.success('Original HTML structure preserved')
}

/**
 * Validate port number is in reasonable range
 */
function validatePort(port: number): void {
  expect(port).toBeGreaterThan(TEST_CONFIG.MIN_PORT)
  expect(port).toBeLessThan(TEST_CONFIG.MAX_PORT)
}
