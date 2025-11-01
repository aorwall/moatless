import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { RepositoryConfig, ServerEvent } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { appContainer } from '../../../container/container'
import { build } from '../../../test/helper'
import { EventService } from '../../events/services/event.service'
import { PreviewOrchestratorService } from '../../preview/services/preview-orchestrator.service'
import { RepositoryService } from '../../repositories/index'
import { WorkspaceCoordinator } from '../services/workspace-coordinator.service'
import { WorkspaceRuntimeStorage } from '../storage/workspace-runtime.storage'

/**
 * Preview Server Lifecycle Integration Test
 *
 * This test verifies the complete preview server functionality through the new
 * workspace runtime architecture:
 * - PreviewOrchestratorService → WorkspaceCoordinator → LocalEnvironmentDriver → PreviewServer
 *
 * Test covers:
 * 1. Repository setup from template (hello-world-ui)
 * 2. Git clone to temporary test data directory
 * 3. Workspace provisioning via WorkspaceCoordinator
 * 4. Setup command execution (bun install)
 * 5. Preview server startup (bun run preview)
 * 6. HTTP accessibility verification
 * 7. Proxy functionality
 * 8. Snapshot state management
 * 9. Graceful shutdown
 */

/**
 * Helper function to collect server events during test execution
 * Subscribes to events for a specific task and collects them in an array
 */
function createEventCollector(eventService: EventService, taskId: string) {
  const events: ServerEvent[] = []
  const unsubscribe = eventService.subscribe(taskId, (event) => {
    if (event.type === 'server') {
      events.push(event as ServerEvent)
    }
  })
  return { events, unsubscribe }
}

// @vitest-environment node
// Run tests sequentially to avoid bun install EEXIST race conditions
describe('Preview Server Lifecycle Integration Test', { concurrent: false }, () => {
  let app: any
  let workspaceCoordinator: WorkspaceCoordinator
  let previewOrchestrator: PreviewOrchestratorService
  let runtimeStorage: WorkspaceRuntimeStorage
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
    runtimeStorage = appContainer.get(WorkspaceRuntimeStorage)
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
      const snapshot = await runtimeStorage.load(taskId)
      workspacePath = snapshot?.executionTarget.workspacePath
    } catch (_error) {
      // Snapshot might not exist
    }

    // Cleanup: Stop preview server
    try {
      await previewOrchestrator.stop(taskId)
      await new Promise((resolve) => setTimeout(resolve, 1000))
    } catch (_error) {
      // Server might not be running
    }

    // Cleanup: Delete runtime snapshot
    try {
      await runtimeStorage.delete(taskId)
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
    await new Promise((resolve) => setTimeout(resolve, 5000))
  })

  it('should complete workspace provisioning and preview state management', async () => {
    // STEP 1: Ensure workspace (triggers git clone + provisioning)
    const handle = await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

    expect(handle.workspacePath).toBeDefined()
    expect(handle.workspacePath).toContain('/workspaces/clones')

    // Verify workspace directory exists
    const workspaceExists = await fs
      .access(handle.workspacePath)
      .then(() => true)
      .catch(() => false)
    expect(workspaceExists).toBe(true)

    // STEP 2: Verify snapshot created
    const snapshot = await runtimeStorage.load(taskId)
    expect(snapshot).toBeDefined()
    expect(snapshot?.repositoryId).toBe(repositoryId)
    expect(snapshot?.executionTarget.mode).toBe('local')
    expect(snapshot?.executionTarget.workspacePath).toContain('/workspaces/clones')

    // STEP 3: Verify package.json exists (git clone succeeded)
    const packageJsonPath = path.join(handle.workspacePath, 'package.json')
    const packageJsonExists = await fs
      .access(packageJsonPath)
      .then(() => true)
      .catch(() => false)
    expect(packageJsonExists).toBe(true)

    // STEP 4: Verify preview state in snapshot
    // Initial state should be either 'none', 'stopped' (from cleanup), or 'starting' (auto-started by WorkspaceCoordinator)
    expect(['none', 'stopped', 'starting']).toContain(snapshot?.preview.status)

    console.log('✓ Workspace provisioning and state management verified')
  }, 30000) // 30 second timeout for git clone

  it('should execute setup commands and create node_modules', async () => {
    // Setup event collector
    const { events, unsubscribe } = createEventCollector(eventService, taskId)

    try {
      // Ensure workspace first
      const handle = await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

      // Start preview (triggers setup commands on first start)
      await previewOrchestrator.start(taskId, repositoryId)

      // Note: Setup commands (bun install) are executed by PreviewOrchestratorService before starting preview
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

      console.log('✓ Server events verified:', {
        startingEvent: startingEvent?.subType,
        startedEvent: startedEvent?.subType,
      })
    } finally {
      unsubscribe()
    }
  }, 90000)

  it('should restart preview server correctly', async () => {
    // Setup event collector
    const { events, unsubscribe } = createEventCollector(eventService, taskId)

    try {
      // Ensure workspace first
      await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

      // Start server
      const initialStatus = await previewOrchestrator.start(taskId, repositoryId)
      expect(initialStatus.status).toBe('started')

      // Clear events before restart
      events.length = 0

      // Restart server
      const restartedStatus = await previewOrchestrator.restart(taskId)

      // Verify new server is running
      expect(restartedStatus.status).toBe('started')
      expect(restartedStatus.url).toBeDefined()

      // Server should be accessible
      const response = await fetch(restartedStatus.url!, {
        signal: AbortSignal.timeout(5000),
      })
      expect(response.ok).toBe(true)

      // Verify restart events: stopped -> starting -> started
      const stoppedEvent = events.find((e) => e.subType === 'stopped')
      const startingEvent = events.find((e) => e.subType === 'starting')
      const startedEvent = events.find((e) => e.subType === 'started')

      expect(stoppedEvent).toBeDefined()
      expect(stoppedEvent?.payload.status).toBe('stopped')

      expect(startingEvent).toBeDefined()
      expect(startingEvent?.payload.status).toBe('starting')

      expect(startedEvent).toBeDefined()
      expect(startedEvent?.payload.status).toBe('started')

      console.log('✓ Restart events verified:', {
        sequence: [stoppedEvent?.subType, startingEvent?.subType, startedEvent?.subType],
      })
    } finally {
      unsubscribe()
    }
  }, 90000)

  it('should handle preview startup errors gracefully', async () => {
    // Setup event collector
    const { events, unsubscribe } = createEventCollector(eventService, taskId)

    // Create repository with invalid start command
    const badRepoConfig: RepositoryConfig = {
      ...repositoryConfig,
      id: `bad-repo-${randomUUID()}`,
      previewConfig: {
        serverStartCommands: ['nonexistent-command-that-will-fail'],
        startupTimeoutSeconds: 5, // Short timeout for test
      },
    }

    let badRepoId: string | undefined

    try {
      // Save bad repository
      await repositoryService.saveRepository(badRepoConfig)
      badRepoId = badRepoConfig.id

      // Ensure workspace first (clone succeeds)
      await workspaceCoordinator.ensureWorkspace(taskId, badRepoId)

      // Attempt to start preview (should return failed status)
      const status = await previewOrchestrator.start(taskId, badRepoId)
      expect(status.status).toBe('failed')
      expect(status.error).toBeDefined()

      // Verify snapshot shows failed state
      const snapshot = await runtimeStorage.load(taskId)
      expect(snapshot?.preview.status).toBe('failed')
      expect(snapshot?.preview.lastError).toBeDefined()

      // Verify failed event was emitted
      const failedEvent = events.find((e) => e.subType === 'failed')
      expect(failedEvent).toBeDefined()
      expect(failedEvent?.type).toBe('server')
      expect(failedEvent?.taskId).toBe(taskId)
      expect(failedEvent?.payload.status).toBe('failed')
      expect(failedEvent?.payload.error).toBeDefined()
      expect(failedEvent?.isError).toBe(true)

      console.log('✓ Failed event verified:', {
        subType: failedEvent?.subType,
        hasError: !!failedEvent?.payload.error,
      })
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
  }, 60000)

  it('should persist preview state in snapshots', async () => {
    // Ensure workspace
    await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

    // Start preview
    const status = await previewOrchestrator.start(taskId, repositoryId)
    expect(status.status).toBe('started')

    // Load snapshot and verify preview state
    const snapshot = await runtimeStorage.load(taskId)
    expect(snapshot?.preview.status).toBe('started')
    expect(snapshot?.preview.endpoint).toBe(status.url)
    expect(snapshot?.preview.lastStartedAt).toBeDefined()

    // Verify snapshot can be reloaded
    const reloadedSnapshot = await runtimeStorage.load(taskId)
    expect(reloadedSnapshot?.preview.status).toBe('started')
    expect(reloadedSnapshot?.preview.endpoint).toBe(status.url)
  }, 90000)

  it('should get preview logs after server starts', async () => {
    // Ensure workspace and start preview
    await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)
    await previewOrchestrator.start(taskId, repositoryId)

    // Get logs
    const logs = await previewOrchestrator.getLogs(taskId, 50)

    // Verify logs contain preview server output
    expect(logs).toBeDefined()
    expect(logs.length).toBeGreaterThan(0)

    // Logs should contain some indication of server activity
    // (exact format depends on vite output)
    expect(typeof logs).toBe('string')
  }, 90000)

  // Helper function to create test repository from template
  async function createTestRepository(): Promise<RepositoryConfig> {
    // Use createRepositoryFromTemplate which:
    // 1. Clones from pre-installed template at /Users/albert/.moatless/templates/hello-world-ui
    // 2. Creates local repository with proper setup/preview commands from TEMPLATES config
    // 3. Uses Bun for all commands (no Node.js v23 issues!)
    const repository = await repositoryService.createRepositoryFromTemplate(
      `hello-world-ui-test-${randomUUID()}`,
      'hello-world-ui',
    )

    // Enable cloneOnNewTask so each task gets a fresh clone from the local template
    repository.cloneOnNewTask = true
    await repositoryService.saveRepository(repository)

    return repository
  }
})
