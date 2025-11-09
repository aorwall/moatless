import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { RepositoryConfig, ServerEvent } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { appContainer } from '../../../container/container.ts'
import { build } from '../../../test/helper.ts'
import { EventService } from '../../events/services/event.service.ts'
import { RepositoryService } from '../../repositories'
import { WorkspaceRuntimeRepository } from '../../workspace/db/workspace-runtime.repository.ts'
import { WorkspaceCoordinator } from '../../workspace/services/workspace-coordinator.service.ts'
import { PreviewOrchestratorService } from '../services/preview-orchestrator.service.ts'

/**
 * Preview Server Lifecycle Integration Test
 *
 * Verifies preview server functionality including workspace provisioning, setup command execution,
 * server lifecycle (start/restart/stop), event streaming, and state management.
 *
 * Tests are organized to minimize bun install race conditions:
 * - Test 1: Full success path lifecycle (single workspace, single bun install)
 * - Test 2: Error handling path (separate workspace with invalid config)
 */

/** Helper to collect server events during test execution */
function createEventCollector(eventService: EventService, taskId: string) {
  const events: ServerEvent[] = []
  const unsubscribe = eventService.subscribe(taskId, (event) => {
    if (event.type === 'server') {
      events.push(event as ServerEvent)
    }
  })
  return { events, unsubscribe }
}

/** Helper to wait for server to reach a specific status */
async function waitForServerStatus(
  previewOrchestrator: PreviewOrchestratorService,
  taskId: string,
  targetStatus: 'started' | 'stopped' | 'failed',
  timeoutMs = 30000,
): Promise<void> {
  const startTime = Date.now()
  while (Date.now() - startTime < timeoutMs) {
    const status = await previewOrchestrator.getStatus(taskId)
    if (status.status === targetStatus) {
      return
    }
    if (status.status === 'failed' && targetStatus !== 'failed') {
      throw new Error(`Server failed to start: ${status.error}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`Timeout waiting for server to reach status: ${targetStatus}`)
}

// @vitest-environment node
describe('Preview Server Lifecycle Integration Test', () => {
  let app: any
  let workspaceCoordinator: WorkspaceCoordinator
  let previewOrchestrator: PreviewOrchestratorService
  let runtimeRepository: WorkspaceRuntimeRepository
  let repositoryService: RepositoryService
  let eventService: EventService

  let taskId: string
  let repositoryId: string
  let repositoryConfig: RepositoryConfig

  beforeEach(async () => {
    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

    // Build app and initialize container
    app = await build()

    // Get services from container (container is initialized by build())
    workspaceCoordinator = appContainer.get(WorkspaceCoordinator)
    previewOrchestrator = appContainer.get(PreviewOrchestratorService)
    runtimeRepository = appContainer.get(WorkspaceRuntimeRepository)
    repositoryService = appContainer.get(RepositoryService)
    eventService = appContainer.get(EventService)

    // Generate unique task ID for isolation
    taskId = `preview-test-${randomUUID()}`

    // Create repository configuration based on hello-world-ui template
    repositoryConfig = await createTestRepository()
    repositoryId = repositoryConfig.id
  })

  afterEach(async () => {
    let workspacePath: string | undefined

    // Get workspace path before deleting snapshot
    try {
      const snapshot = await runtimeRepository.load(taskId)
      workspacePath = snapshot?.executionTarget.workspacePath
    } catch (_error) {
      // Snapshot might not exist
    }

    // Cleanup: Stop preview server
    try {
      await previewOrchestrator.stop(taskId)
      // Increased delay to ensure process termination completes
      await new Promise((resolve) => setTimeout(resolve, 2000))
    } catch (_error) {
      // Server might not be running
    }

    // Cleanup: Delete runtime snapshot
    try {
      await runtimeRepository.delete(taskId)
    } catch (_error) {
      // Snapshot might not exist
    }

    // Cleanup: Remove cloned workspace directory
    if (workspacePath) {
      try {
        await fs.rm(workspacePath, { recursive: true, force: true })
        // Wait for filesystem to complete deletion
        await new Promise((resolve) => setTimeout(resolve, 500))
      } catch (_error) {
        // Workspace might not exist or already deleted
      }
    }

    // Cleanup: Delete test repository
    try {
      if (repositoryId) {
        await repositoryService.deleteRepository(repositoryId)
      }
    } catch (_error) {
      // Repository might not exist
    }

    // Close app
    if (app) {
      await app.close()
    }

    // Additional delay to ensure full cleanup before next test
    // This prevents bun install EEXIST race conditions when tests run sequentially
    // The delay allows bun's global cache/lock to fully release
    await new Promise((resolve) => setTimeout(resolve, 10000))
  })

  it('should complete full preview server lifecycle', async () => {
    // Setup event collector for entire test
    const { events, unsubscribe } = createEventCollector(eventService, taskId)

    try {
      // ===== PHASE 1: Workspace Provisioning =====
      // Ensure workspace (triggers git clone + provisioning)
      // Note: This automatically starts preview server in background
      const handle = await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

      expect(handle.workspacePath).toBeDefined()
      expect(handle.workspacePath).toContain('/workspaces/clones')

      // Verify workspace directory exists
      const workspaceExists = await fs
        .access(handle.workspacePath)
        .then(() => true)
        .catch(() => false)
      expect(workspaceExists).toBe(true)

      // Verify snapshot created
      let snapshot = await runtimeRepository.load(taskId)
      expect(snapshot).toBeDefined()
      expect(snapshot?.repositoryId).toBe(repositoryId)
      expect(snapshot?.executionTarget.mode).toBe('local')
      expect(snapshot?.executionTarget.workspacePath).toContain('/workspaces/clones')

      // Verify package.json exists (git clone succeeded)
      const packageJsonPath = path.join(handle.workspacePath, 'package.json')
      const packageJsonExists = await fs
        .access(packageJsonPath)
        .then(() => true)
        .catch(() => false)
      expect(packageJsonExists).toBe(true)

      // Initial state should be either 'none', 'stopped', or 'starting'
      expect(['none', 'stopped', 'starting']).toContain(snapshot?.preview.status)

      // ===== PHASE 2: Setup & Start =====
      // Wait for background preview server start to complete (triggered by ensureWorkspace)
      // Poll until server status is 'started' or 'failed'
      let retries = 0
      while (retries < 30) {
        snapshot = await runtimeRepository.load(taskId)
        if (snapshot?.preview.status === 'started' || snapshot?.preview.status === 'failed') {
          break
        }
        await new Promise((resolve) => setTimeout(resolve, 1000))
        retries++
      }

      // Get server status after background start completes
      const status = await previewOrchestrator.getStatus(taskId)
      expect(status.status).toBe('started')

      // Verify node_modules exists (proof that bun install ran)
      const nodeModulesPath = path.join(handle.workspacePath, 'node_modules')
      const nodeModulesExists = await fs
        .access(nodeModulesPath)
        .then(() => true)
        .catch(() => false)
      expect(nodeModulesExists).toBe(true)

      // Verify node_modules is not empty
      const files = await fs.readdir(nodeModulesPath)
      expect(files.length).toBeGreaterThan(0)

      // Verify server events were emitted
      const startingEvent = events.find((e) => e.subType === 'starting')
      const startedEvent = events.find((e) => e.subType === 'started')

      expect(startingEvent).toBeDefined()
      expect(startingEvent?.type).toBe('server')
      expect(startingEvent?.taskId).toBe(taskId)
      expect(startingEvent?.payload.status).toBe('starting')

      expect(startedEvent).toBeDefined()
      expect(startedEvent?.type).toBe('server')
      expect(startedEvent?.taskId).toBe(taskId)
      expect(startedEvent?.payload.status).toBe('started')
      expect(startedEvent?.payload.port).toBeDefined()
      expect(startedEvent?.payload.url).toBeDefined()

      // ===== PHASE 3: Snapshot Persistence =====
      // Load snapshot and verify preview state
      snapshot = await runtimeRepository.load(taskId)
      expect(snapshot?.preview.status).toBe('started')
      expect(snapshot?.preview.proxyUrl).toBe(status.url) // status.url returns proxyUrl
      expect(snapshot?.preview.lastStartedAt).toBeDefined()

      // Verify snapshot can be reloaded
      const reloadedSnapshot = await runtimeRepository.load(taskId)
      expect(reloadedSnapshot?.preview.status).toBe('started')
      expect(reloadedSnapshot?.preview.proxyUrl).toBe(status.url)

      // ===== PHASE 4: Log Retrieval =====
      // Get logs from running server
      const logs = await previewOrchestrator.getLogs(taskId, 50)

      // Verify logs contain preview server output
      expect(logs).toBeDefined()
      expect(logs.length).toBeGreaterThan(0)
      expect(typeof logs).toBe('string')

      // ===== PHASE 5: Restart =====
      // Clear events before restart
      events.length = 0

      // Restart server (returns void - starts in background)
      await previewOrchestrator.restart(taskId)

      // Wait for server to reach "started" status
      await waitForServerStatus(previewOrchestrator, taskId, 'started')

      // Get final status
      const finalStatus = await previewOrchestrator.getStatus(taskId)
      expect(finalStatus.status).toBe('started')
      expect(finalStatus.url).toBeDefined()

      // Server should be accessible
      const response = await fetch(finalStatus.url!, {
        signal: AbortSignal.timeout(5000),
      })
      expect(response.ok).toBe(true)

      // Verify restart events: stopped -> starting -> started
      const stoppedEvent = events.find((e) => e.subType === 'stopped')
      const restartStartingEvent = events.find((e) => e.subType === 'starting')
      const restartStartedEvent = events.find((e) => e.subType === 'started')

      expect(stoppedEvent).toBeDefined()
      expect(stoppedEvent?.payload.status).toBe('stopped')

      expect(restartStartingEvent).toBeDefined()
      expect(restartStartingEvent?.payload.status).toBe('starting')

      expect(restartStartedEvent).toBeDefined()
      expect(restartStartedEvent?.payload.status).toBe('started')
    } finally {
      unsubscribe()
    }
  }, 120000) // 2 minute timeout for full lifecycle

  it('should handle preview startup errors gracefully', async () => {
    // Setup event collector
    const { events, unsubscribe } = createEventCollector(eventService, taskId)

    // Create repository with invalid start command
    // Override codeExecution.setupCommands to skip bun install
    const badRepoConfig: RepositoryConfig = {
      ...repositoryConfig,
      id: `bad-repo-${randomUUID()}`,
      previewConfig: {
        ...repositoryConfig.previewConfig,
        serverStartCommands: ['nonexistent-command-that-will-fail'],
        startupTimeoutSeconds: 5, // Short timeout for test
      },
      codeExecution: {
        ...repositoryConfig.codeExecution,
        setupCommands: [], // Skip bun install - not needed for error handling test
      },
    }

    let badRepoId: string | undefined

    try {
      // Save bad repository
      await repositoryService.saveRepository(badRepoConfig)
      badRepoId = badRepoConfig.id

      // Ensure workspace first (clone succeeds)
      // Note: ensureWorkspace() automatically starts preview server in background
      // because badRepoConfig has serverStartCommands configured
      await workspaceCoordinator.ensureWorkspace(taskId, badRepoId)

      // Wait for server to fail (command should fail immediately with Promise.race())
      // No need to check initial 'starting' status since start is async and might race
      await waitForServerStatus(previewOrchestrator, taskId, 'failed', 10000)

      // Verify final failed status
      const finalStatus = await previewOrchestrator.getStatus(taskId)
      expect(finalStatus.status).toBe('failed')
      expect(finalStatus.error).toBeDefined()

      // Verify failed event was emitted
      const failedEvent = events.find((e) => e.subType === 'failed')
      expect(failedEvent).toBeDefined()
      expect(failedEvent?.type).toBe('server')
      expect(failedEvent?.taskId).toBe(taskId)
      expect(failedEvent?.payload.status).toBe('failed')
      expect(failedEvent?.payload.error).toBeDefined()
      expect(failedEvent?.isError).toBe(true)
    } finally {
      unsubscribe()
      // Cleanup bad repository if it was created
      if (badRepoId) {
        try {
          await repositoryService.deleteRepository(badRepoId)
        } catch (_error) {
          // Repository might not exist
        }
      }
    }
  }, 30000)

  // Helper function to create test repository from template
  async function createTestRepository(): Promise<RepositoryConfig> {
    // Use createLocalRepositoryFromTemplate which:
    // 1. Clones from pre-installed template at /Users/albert/.moatless/templates/hello-world-ui
    // 2. Creates local repository with proper setup/preview commands from TEMPLATES config
    // 3. Uses Bun for all commands (no Node.js v23 issues!)
    const repository = await repositoryService.createLocalRepositoryFromTemplate({
      kind: 'local',
      name: `hello-world-ui-test-${randomUUID()}`,
      templateId: 'hello-world-ui',
    })

    // Template repositories use their path directly (or create worktrees based on useGitWorktrees flag)
    // No need to enable cloneOnNewTask - workspace mode is determined by repository kind
    return repository
  }
})
