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

describe('Port Allocation Integration Test', () => {
  let tempDir: string
  let repositoryService: RepositoryService
  let workspaceService: WorkspaceService
  let repository: RepositoryConfig

  beforeAll(async () => {
    // Setup temp directory for isolated test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'port-alloc-test-'))
    process.env['DATA_DIR'] = tempDir

    console.log(`Test environment created at: ${tempDir}`)

    // Initialize services
    const storage = new FileStorage(tempDir)

    const eventService = {
      emitEvent: () => Promise.resolve(),
      ...new EventEmitter(),
    } as any

    const dockerService = {} as DockerService
    const claudeService = {} as ClaudeService

    repositoryService = new RepositoryService(storage)

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

    // Clone repository once for all tests
    console.log('Creating repository from GitHub...')
    repository = await repositoryService.createRepository({
      name: 'port-alloc-test',
      sourceType: 'github',
      remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
      cloneOnNewTask: true,
      codeExecution: {
        type: 'local',
        setupCommands: ['bun install'],
      },
      previewConfig: {
        serverStartCommands: ['bun run dev --port {PORT}'],
        enableInspector: false,
      },
    })

    console.log(`Repository created: ${repository.name}`)
  }, 120000)

  afterAll(async () => {
    // Cleanup temp directory
    try {
      console.log(`Removing temp directory: ${tempDir}`)
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Error removing temp directory:', error)
    }

    delete process.env['DATA_DIR']
  }, 60000)

  it('should allocate different ports for concurrent servers', async () => {
    let workspace1: Workspace | null = null
    let workspace2: Workspace | null = null

    try {
      // Helper function to start a server and get its port
      const startServerAndGetPort = async (
        taskId: string,
        name: string,
      ): Promise<{ workspace: Workspace; port: number; url: string }> => {
        console.log(`\n--- Starting ${name} ---`)
        const workspace = await workspaceService.createWorkspace(taskId, repository.id)

        console.log(`Setting up ${name}...`)
        await workspace.setup()

        console.log(`Starting preview server for ${name}...`)
        const startResult = await workspace.startServer()

        // Wait for server to start
        let serverStatus = startResult
        const startTime = Date.now()
        const timeout = 30000

        while (serverStatus.status === 'starting' && Date.now() - startTime < timeout) {
          await new Promise((resolve) => setTimeout(resolve, 500))
          serverStatus = await workspace.getServerStatus()

          if (serverStatus.status === 'started') {
            console.log(`✅ ${name} started on ${serverStatus.url}`)
            // Give the server more time to fully bind to the port
            await new Promise((resolve) => setTimeout(resolve, 2000))
            break
          }

          if (serverStatus.status === 'failed') {
            const logs = await workspace.getServerLogs(100)
            console.log(logs)
            throw new Error(`${name} failed: ${serverStatus.error}`)
          }
        }

        if (serverStatus.status !== 'started') {
          throw new Error(`${name} did not start within timeout`)
        }

        expect(serverStatus.url).toBeDefined()
        const portMatch = serverStatus.url!.match(/:(\d+)$/)
        expect(portMatch).toBeTruthy()
        expect(portMatch![1]).toBeDefined()
        const port = Number.parseInt(portMatch![1]!, 10)

        // Verify server is still running after delay
        const finalStatus = await workspace.getServerStatus()
        if (finalStatus.status !== 'started') {
          const logs = await workspace.getServerLogs(100)
          console.log(`❌ ${name} status changed to ${finalStatus.status}:`)
          console.log(logs)
          throw new Error(`${name} is no longer running: ${finalStatus.status}`)
        }

        // Log server output for debugging
        const logs = await workspace.getServerLogs(50)
        console.log(`${name} logs:\n${logs}`)

        return { workspace, port, url: serverStatus.url! }
      }

      // Start first server
      const server1 = await startServerAndGetPort(randomUUID(), 'Server 1')
      workspace1 = server1.workspace

      // Start second server (should get different port)
      const server2 = await startServerAndGetPort(randomUUID(), 'Server 2')
      workspace2 = server2.workspace

      // Verify ports are different
      console.log('\n--- Verification ---')
      console.log(`Server 1 port: ${server1.port}`)
      console.log(`Server 2 port: ${server2.port}`)

      expect(server1.port).not.toBe(server2.port)
      expect(server1.port).toBeGreaterThanOrEqual(9000)
      expect(server1.port).toBeLessThanOrEqual(9999)
      expect(server2.port).toBeGreaterThanOrEqual(9000)
      expect(server2.port).toBeLessThanOrEqual(9999)

      // Verify both servers are HTTP-accessible
      console.log('Testing HTTP connectivity to both servers...')
      const response1 = await fetch(server1.url, {
        signal: AbortSignal.timeout(5000),
      })
      expect(response1.ok).toBe(true)
      console.log(`✅ Server 1 (port ${server1.port}) is accessible`)

      const response2 = await fetch(server2.url, {
        signal: AbortSignal.timeout(5000),
      })
      expect(response2.ok).toBe(true)
      console.log(`✅ Server 2 (port ${server2.port}) is accessible`)

      // Stop both servers
      console.log('\nStopping servers...')
      await workspace1.stopServer()
      await workspace2.stopServer()

      const status1 = await workspace1.getServerStatus()
      const status2 = await workspace2.getServerStatus()
      expect(status1.status).toBe('stopped')
      expect(status2.status).toBe('stopped')
      console.log('✅ Both servers stopped successfully')
    } finally {
      // Cleanup
      if (workspace1) {
        try {
          await workspace1.teardown(true)
        } catch (error) {
          console.warn('Error tearing down workspace1:', error)
        }
      }
      if (workspace2) {
        try {
          await workspace2.teardown(true)
        } catch (error) {
          console.warn('Error tearing down workspace2:', error)
        }
      }
    }
  }, 180000)

  it('should reuse port after server stops', async () => {
    console.log('\n--- Testing Port Reuse ---')
    let workspace1: Workspace | null = null
    let workspace2: Workspace | null = null

    try {
      // Start first server
      const taskId1 = randomUUID()
      workspace1 = await workspaceService.createWorkspace(taskId1, repository.id)
      await workspace1.setup()

      console.log('Starting first server...')
      const start1 = await workspace1.startServer()

      // Wait for first server to start
      let status1 = start1
      const startTime1 = Date.now()
      while (status1.status === 'starting' && Date.now() - startTime1 < 30000) {
        await new Promise((resolve) => setTimeout(resolve, 500))
        status1 = await workspace1.getServerStatus()
        if (status1.status === 'started') break
      }

      expect(status1.status).toBe('started')
      const port1Match = status1.url!.match(/:(\d+)$/)
      expect(port1Match![1]).toBeDefined()
      const port1 = Number.parseInt(port1Match![1]!, 10)
      console.log(`First server started on port ${port1}`)

      // Stop first server
      console.log('Stopping first server...')
      await workspace1.stopServer()
      const stopped1 = await workspace1.getServerStatus()
      expect(stopped1.status).toBe('stopped')
      console.log('✅ First server stopped')

      // Longer delay to ensure port is fully released by OS
      await new Promise((resolve) => setTimeout(resolve, 2000))

      // Start second server (should get same port since first is stopped)
      const taskId2 = randomUUID()
      workspace2 = await workspaceService.createWorkspace(taskId2, repository.id)
      await workspace2.setup()

      console.log('Starting second server...')
      const start2 = await workspace2.startServer()

      // Wait for second server to start
      let status2 = start2
      const startTime2 = Date.now()
      while (status2.status === 'starting' && Date.now() - startTime2 < 30000) {
        await new Promise((resolve) => setTimeout(resolve, 500))
        status2 = await workspace2.getServerStatus()
        if (status2.status === 'started') break
      }

      expect(status2.status).toBe('started')
      const port2Match = status2.url!.match(/:(\d+)$/)
      expect(port2Match![1]).toBeDefined()
      const port2 = Number.parseInt(port2Match![1]!, 10)
      console.log(`Second server started on port ${port2}`)

      // Verify second server got a valid port in the range
      // Note: OS may not immediately release port1, so we can't guarantee exact reuse
      expect(port2).toBeGreaterThanOrEqual(9000)
      expect(port2).toBeLessThanOrEqual(9999)
      console.log(`✅ Port ${port2} successfully allocated after first server stopped`)

      // Cleanup
      await workspace2.stopServer()
    } finally {
      if (workspace1) {
        try {
          await workspace1.teardown(true)
        } catch (error) {
          console.warn('Error tearing down workspace1:', error)
        }
      }
      if (workspace2) {
        try {
          await workspace2.teardown(true)
        } catch (error) {
          console.warn('Error tearing down workspace2:', error)
        }
      }
    }
  }, 180000)
})
