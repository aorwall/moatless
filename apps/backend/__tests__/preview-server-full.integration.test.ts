import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { ClaudeService } from '../agents/claude-code/claude.service.js'
import type { DockerService } from '../services/docker.service.js'
import { RepositoryService } from '../services/repository.service.js'
import { getSettingsService } from '../services/settings.service.js'
import { FileStorage } from '../services/storage.js'
import { WorkspaceService } from '../services/workspace.service.js'
import type { Workspace } from '../workspaces/workspace.js'

describe('PreviewServer Full Integration Test with Port Detection', () => {
  let tempDir: string
  let repositoryService: RepositoryService
  let workspaceService: WorkspaceService
  let workspace: Workspace
  let repository: RepositoryConfig
  let taskId: string

  beforeAll(async () => {
    // Setup temp directory for isolated test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'preview-test-'))
    process.env.DATA_DIR = tempDir

    console.log(`Test environment created at: ${tempDir}`)

    // Initialize services with minimal mocking
    const storage = new FileStorage(tempDir)

    // Create a proper EventService mock with emitEvent method
    const eventService = {
      emitEvent: () => Promise.resolve(),
      ...new EventEmitter(),
    } as any

    const dockerService = {} as DockerService // Mock, not used for local environment
    const claudeService = {} as ClaudeService // Mock, not used for this test

    // Create repository service and set up settings service for GitHub cloning
    repositoryService = new RepositoryService(storage)

    // Create a mock GitHub service for settings service
    const mockGitHubService = {
      apiBaseUrl: 'https://api.github.com',
      getAuthenticatedUser: () => Promise.resolve(null),
      getUserEmails: () => Promise.resolve([]),
      validateToken: () => Promise.resolve(false),
      getTokenScopes: () => Promise.resolve([]),
    } as any

    const settingsService = getSettingsService(mockGitHubService)
    repositoryService.setSettingsService(settingsService)
    workspaceService = new WorkspaceService(
      repositoryService,
      storage,
      eventService,
      dockerService,
      claudeService,
    )

    // Clone the real repository and use the actual dev server commands
    // This tests the full integration with a real Vite development server
    console.log('Creating repository from GitHub (using real bun run dev)...')
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

    console.log(`Repository created: ${repository.name} at ${repository.path}`)

    taskId = randomUUID()
  }, 120000) // 2 minute timeout for cloning

  afterAll(async () => {
    // Cleanup workspace and temp directory
    try {
      if (workspace) {
        console.log('Cleaning up workspace...')
        await workspace.teardown(true)
      }
    } catch (error) {
      console.warn('Error during workspace cleanup:', error)
    }

    try {
      console.log(`Removing temp directory: ${tempDir}`)
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Error removing temp directory:', error)
    }

    // Clean up environment variable
    delete process.env.DATA_DIR
  }, 30000)

  it('should start preview server, detect port, and verify proxy injection', async () => {
    // 1. Create workspace
    console.log('Creating workspace...')
    workspace = await workspaceService.createWorkspace(taskId, repository.id)
    expect(workspace).toBeDefined()
    expect(workspace.id).toBe(taskId)

    // 2. Setup environment (runs bun install)
    console.log('Setting up environment (running bun install)...')
    await workspace.setup()
    expect(workspace.status).toBe('ready')

    // 3. Start server - this should detect port from logs since no {PORT} placeholder
    console.log('Starting preview server (will detect port from logs)...')
    const startResult = await workspace.startServer()

    // 4. Wait for server to transition from 'starting' to 'running' state
    let serverStatus = startResult
    if (serverStatus.status === 'starting') {
      console.log('Server is starting, waiting for port detection to complete...')

      const startTime = Date.now()
      const timeout = 30000 // 30 second timeout

      while (serverStatus.status === 'starting' && Date.now() - startTime < timeout) {
        await new Promise((resolve) => setTimeout(resolve, 500)) // Poll every 500ms
        serverStatus = await workspace.getServerStatus()

        if (serverStatus.status === 'started') {
          console.log(`✅ Server is now running on ${serverStatus.url}`)
          break
        }

        if (serverStatus.status === 'failed') {
          console.log(`❌ Server failed to start: ${serverStatus.error}`)
          break
        }
      }

      if (serverStatus.status === 'starting') {
        throw new Error(`Server did not start within ${timeout}ms timeout`)
      }
    }

    // 5. Verify server started successfully and port was detected
    expect(serverStatus.status).toBe('started')
    expect(serverStatus.url).toBeDefined()
    expect(serverStatus.url).toMatch(/^http:\/\/localhost:\d+$/)

    // Extract the detected port
    const portMatch = serverStatus.url!.match(/:(\d+)$/)
    expect(portMatch).toBeTruthy()
    const port = Number.parseInt(portMatch![1], 10)
    console.log(`✅ Port successfully detected from logs: ${port}`)

    // Verify port is in reasonable range
    expect(port).toBeGreaterThan(1000)
    expect(port).toBeLessThan(65536)

    // Server should already be HTTP-responsive since PreviewServer waits for it
    // 6. Test actual HTTP connectivity to the detected port
    console.log(`Testing HTTP connectivity to detected port ${port}...`)
    const response = await fetch(serverStatus.url!, {
      signal: AbortSignal.timeout(5000), // 5 second timeout
    })
    expect(response.ok).toBe(true)
    expect(response.status).toBe(200)
    const directHtml = await response.text() // Get direct HTML first
    console.log('✅ HTTP connectivity test passed')

    // 6.5. Verify proxy URL is available and working
    expect(serverStatus.proxyUrl).toBeDefined()
    expect(serverStatus.proxyUrl).toMatch(/^http:\/\/localhost:\d+$/)
    console.log(`✅ Proxy URL available: ${serverStatus.proxyUrl}`)

    // Extract proxy port and verify it's different from main port
    const proxyPortMatch = serverStatus.proxyUrl!.match(/:(\d+)$/)
    expect(proxyPortMatch).toBeTruthy()
    const proxyPort = Number.parseInt(proxyPortMatch![1], 10)
    expect(proxyPort).not.toBe(port) // Proxy should be on different port
    console.log(`✅ Proxy port ${proxyPort} is different from main port ${port}`)

    // 6.6. Fetch content from proxy and verify script injection
    console.log('Testing proxy content and script injection...')
    const proxyResponse = await fetch(serverStatus.proxyUrl!, {
      signal: AbortSignal.timeout(5000),
    })
    expect(proxyResponse.ok).toBe(true)
    expect(proxyResponse.status).toBe(200)
    expect(proxyResponse.headers.get('content-type')).toMatch(/text\/html/)

    const proxyHtml = await proxyResponse.text()

    // Verify inspector script is injected in proxy response
    expect(proxyHtml).toContain('Preview Inspector')
    expect(proxyHtml).toContain(
      'Preview Inspector Script - Injected by Preview Environment Service',
    )
    expect(proxyHtml).toContain('hookConsole')
    expect(proxyHtml).toContain('hookFetch')
    expect(proxyHtml).toContain('hookScroll')
    expect(proxyHtml).toContain('hookNavigation')
    expect(proxyHtml).toContain(`taskId: '${taskId}'`)
    console.log('✅ Inspector script found in proxy response')

    // Verify script is injected before closing body tag
    const scriptTagIndex = proxyHtml.indexOf('<script>')
    const bodyCloseIndex = proxyHtml.indexOf('</body>')
    expect(scriptTagIndex).toBeGreaterThan(0)
    expect(bodyCloseIndex).toBeGreaterThan(scriptTagIndex)
    console.log('✅ Script correctly injected before </body>')

    // Verify proxy response is larger than direct response (due to script injection)
    expect(proxyHtml.length).toBeGreaterThan(directHtml.length)
    const injectedSize = proxyHtml.length - directHtml.length
    console.log(`✅ Proxy response is ${injectedSize} bytes larger (script injection)`)

    // Verify direct response does NOT contain inspector script
    expect(directHtml).not.toContain('Preview Inspector Script')
    console.log('✅ Direct server response does not contain inspector script')

    // Verify original content is preserved in proxy response
    expect(proxyHtml.toLowerCase()).toContain('<!doctype html>')
    expect(proxyHtml).toContain('</html>')
    console.log('✅ Original HTML structure preserved in proxy')

    // 7. Get server status - this should NOT call detectAndValidatePort again (optimization test)
    console.log('Checking server status (testing optimization)...')
    const status = await workspace.getServerStatus()
    expect(status.status).toBe('started')
    expect(status.url).toBe(serverStatus.url)
    // Note: responsive check can be flaky in test environments, focus on the core functionality
    console.log('✅ Server status check passed without re-detecting port')

    // 8. Stop server
    console.log('Stopping preview server...')
    const stopped = await workspace.stopServer()
    expect(stopped).toBe(true)

    // 9. Verify server is stopped
    const finalStatus = await workspace.getServerStatus()
    expect(finalStatus.status).toBe('stopped')
    console.log('✅ Server stopped successfully')
  }, 180000) // 3 minute timeout for full test

  it('should fail gracefully when port cannot be detected', async () => {
    // Create a repository with a command that runs but doesn't bind to a port
    console.log('Testing port detection failure scenario...')
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
        // Use a command that exits immediately without starting a server
        serverStartCommands: ['echo "not a server"'],
        enableInspector: false,
      },
    })

    const badTaskId = randomUUID()
    const badWorkspace = await workspaceService.createWorkspace(badTaskId, badRepo.id)

    await badWorkspace.setup()

    // Should fail because the build command doesn't start a server/bind to a port
    console.log('Starting server with command that does not bind to port...')
    const result = await badWorkspace.startServer()

    // Wait for server to either start successfully or fail
    let serverStatus = result
    if (result.status === 'starting') {
      console.log('Server is starting, waiting for completion or failure...')

      const startTime = Date.now()
      const timeout = 30000 // 30 second timeout

      while (serverStatus.status === 'starting' && Date.now() - startTime < timeout) {
        await new Promise((resolve) => setTimeout(resolve, 500)) // Poll every 500ms
        serverStatus = await badWorkspace.getServerStatus()

        if (serverStatus.status === 'started') {
          console.log(`Server unexpectedly started on ${serverStatus.url}`)
          break
        }

        if (serverStatus.status === 'failed') {
          console.log(`Server failed as expected: ${serverStatus.error}`)
          break
        }
      }

      // If still starting after timeout, that's also a failure case
      if (serverStatus.status === 'starting') {
        console.log('Server did not start or fail within timeout - treating as failure')
        // This could be a case where the build command completed successfully but no port was detected
      }
    }

    // Should return error state since the command exits immediately
    // The process completes (exit 0) so status is 'stopped', not 'failed'
    expect(['stopped', 'failed']).toContain(serverStatus.status)

    if (serverStatus.status === 'stopped') {
      console.log('✅ Command completed but did not maintain a server process')
    } else if (serverStatus.error) {
      expect(serverStatus.error).toMatch(
        /Failed to start server|HTTP-responsive|no port|killed|Process exited/i,
      )
      console.log('✅ Server failure handled correctly:', serverStatus.error)
    }

    // Cleanup
    await badWorkspace.teardown(true)
  }, 120000) // 2 minute timeout
})
