import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { appContainer } from '../../../container/container'
import { build } from '../../../test/helper'
import { RepositoryService } from '../../repositories/index'
import { PreviewManager } from '../services/preview-manager.service'
import { WorkspaceCoordinator } from '../services/workspace-coordinator.service'
import { WorkspaceRuntimeStorage } from '../storage/workspace-runtime.storage'

/**
 * Preview Server Lifecycle Integration Test
 *
 * This test verifies the complete preview server functionality through the new
 * workspace runtime architecture:
 * - PreviewManager → WorkspaceCoordinator → LocalEnvironmentDriver → PreviewServer
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

describe('Preview Server Lifecycle Integration Test', () => {
  let app: any
  let workspaceCoordinator: WorkspaceCoordinator
  let previewManager: PreviewManager
  let runtimeStorage: WorkspaceRuntimeStorage
  let repositoryService: RepositoryService

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
    previewManager = appContainer.get(PreviewManager)
    runtimeStorage = appContainer.get(WorkspaceRuntimeStorage)
    repositoryService = appContainer.get(RepositoryService)

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
      await previewManager.stop(taskId)
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
  })

  it('should complete full preview lifecycle: clone → setup → start → proxy → stop', async () => {
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

    // STEP 4: Start preview server via PreviewManager
    const status = await previewManager.start(taskId, repositoryId)

    // Verify server started
    expect(status.status).toBe('started')
    expect(status.url).toBeDefined()

    console.log(`Preview server started at ${status.url}`)

    // STEP 5: Verify snapshot updated with preview state
    const updatedSnapshot = await runtimeStorage.load(taskId)
    expect(updatedSnapshot?.preview.status).toBe('started')
    expect(updatedSnapshot?.preview.endpoint).toBeDefined()
    expect(updatedSnapshot?.preview.lastStartedAt).toBeDefined()

    // STEP 6: Test HTTP accessibility
    const response = await fetch(status.url!, {
      signal: AbortSignal.timeout(5000),
    })
    expect(response.ok).toBe(true)
    expect(response.status).toBe(200)

    const html = await response.text()
    expect(html).toContain('html') // Basic HTML check

    // STEP 7: Test preview proxy
    const proxyResponse = await previewManager.proxyRequest(taskId, {
      method: 'GET',
      path: '/',
      headers: new Headers(),
    })
    expect(proxyResponse.status).toBe(200)

    // STEP 8: Check server status
    const currentStatus = await previewManager.getStatus(taskId)
    expect(currentStatus.status).toBe('started')
    expect(currentStatus.url).toBe(status.url)

    // STEP 9: Stop server
    await previewManager.stop(taskId)

    // STEP 10: Verify snapshot updated
    const finalSnapshot = await runtimeStorage.load(taskId)
    expect(finalSnapshot?.preview.status).toBe('stopped')
    expect(finalSnapshot?.preview.lastStoppedAt).toBeDefined()

    // STEP 11: Verify server is no longer accessible
    await expect(
      fetch(status.url!, {
        signal: AbortSignal.timeout(2000),
      }),
    ).rejects.toThrow()
  }, 90000) // 90 second timeout for git clone + bun install + preview start

  it('should execute setup commands and create node_modules', async () => {
    // Ensure workspace first
    const handle = await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

    // Start preview (triggers setup commands on first start)
    await previewManager.start(taskId, repositoryId)

    // Note: Setup commands (bun install) are executed by PreviewManager before starting preview
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
  }, 90000)

  it('should restart preview server correctly', async () => {
    // Ensure workspace first
    await workspaceCoordinator.ensureWorkspace(taskId, repositoryId)

    // Start server
    const initialStatus = await previewManager.start(taskId, repositoryId)
    expect(initialStatus.status).toBe('started')

    // Restart server
    const restartedStatus = await previewManager.restart(taskId)

    // Verify new server is running
    expect(restartedStatus.status).toBe('started')
    expect(restartedStatus.url).toBeDefined()

    // Server should be accessible
    const response = await fetch(restartedStatus.url!, {
      signal: AbortSignal.timeout(5000),
    })
    expect(response.ok).toBe(true)
  }, 90000)

  it('should handle preview startup errors gracefully', async () => {
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
      const created = await repositoryService.createRepository(badRepoConfig)
      badRepoId = created.id

      // Ensure workspace first (clone succeeds)
      await workspaceCoordinator.ensureWorkspace(taskId, badRepoId)

      // Attempt to start preview (should return failed status)
      const status = await previewManager.start(taskId, badRepoId)
      expect(status.status).toBe('failed')
      expect(status.error).toBeDefined()

      // Verify snapshot shows failed state
      const snapshot = await runtimeStorage.load(taskId)
      expect(snapshot?.preview.status).toBe('failed')
      expect(snapshot?.preview.lastError).toBeDefined()
    } finally {
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
    const status = await previewManager.start(taskId, repositoryId)
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
    await previewManager.start(taskId, repositoryId)

    // Get logs
    const logs = await previewManager.getLogs(taskId, 50)

    // Verify logs contain preview server output
    expect(logs).toBeDefined()
    expect(logs.length).toBeGreaterThan(0)

    // Logs should contain some indication of server activity
    // (exact format depends on vite output)
    expect(typeof logs).toBe('string')
  }, 90000)

  // Helper function to create test repository
  async function createTestRepository(): Promise<RepositoryConfig> {
    const testRepoConfig: RepositoryConfig = {
      id: `test-repo-${randomUUID()}`,
      name: 'hello-world-ui-test',
      description: 'Test repository for preview server integration test',
      path: '', // Will be set during clone
      remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
      sourceType: 'github',
      cloneOnNewTask: true, // Clone fresh for each task
      useGitWorktrees: false, // Don't use worktrees - direct clone
      codeExecution: {
        setupCommands: ['bun install'],
      },
      previewConfig: {
        serverStartCommands: ['bun run preview'],
        buildCommand: 'bun run build',
        takeScreenshots: false,
        enableInspector: false,
        startupTimeoutSeconds: 60,
      },
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    // Create repository in the system
    return await repositoryService.createRepository(testRepoConfig)
  }
})
