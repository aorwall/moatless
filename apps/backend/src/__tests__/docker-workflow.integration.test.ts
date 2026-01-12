/**
 * Docker Workflow Integration Tests
 *
 * Tests the complete Docker workspace provisioning flow through the backend:
 * - Repository creation with Docker workspace configuration
 * - Task creation triggering Docker container provisioning
 * - File/Git operations via RemoteEnvironmentDriver
 * - Preview server management
 * - Agent execution in Docker container
 *
 * Prerequisites:
 * - Docker daemon must be running
 * - Sandbox image: ghcr.io/aorwall/moatless-vibe-sandbox-js:latest
 * - Claude authentication (ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN) for agent tests
 *
 * Run: bun test docker-workflow.integration
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Task } from '@moatless/types'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import type { WorkspaceCoordinator } from '../features/workspace/index.js'
import { build } from '../test/helper.js'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'
import {
  hasClaudeAuth,
  isDockerAvailable,
  isPlatformCompatible,
  waitForAgentIdle,
  waitForPreviewReady,
  waitForWorkspaceReady,
} from './utils/docker-helpers.js'

// Check prerequisites before running tests
const dockerAvailable = await isDockerAvailable()
const platformCompatible = isPlatformCompatible()
const hasAuth = hasClaudeAuth()
const canRunTests = dockerAvailable && platformCompatible && hasAuth

describe.skipIf(!canRunTests)('Docker Workflow Integration Test', () => {
  let app: ReturnType<typeof build> extends Promise<infer T> ? T : never
  let workspaceCoordinator: WorkspaceCoordinator
  let createdRepositoryId: string
  let createdTaskId: string
  let tempDir: string

  beforeAll(async () => {
    // Create a temporary directory for test data (fresh database)
    const dataDir = getDataDir()
    tempDir = await fs.mkdtemp(path.join(dataDir, 'docker-workflow-test-'))

    // Set DATA_DIR to the temp directory so storage uses it
    process.env['DATA_DIR'] = tempDir

    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

    // Force Docker workspace provisioner (this controls which provisioner is used)
    process.env['WORKSPACE_EXECUTION_MODE'] = 'docker'

    // Build test app
    app = await build()
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Get services
    workspaceCoordinator = app.services.workspaceCoordinator

    if (!workspaceCoordinator) {
      throw new Error('WorkspaceCoordinator service not available')
    }

    // Create repository for all tests
    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: {
        kind: 'remote',
        name: `docker-test-${Date.now()}`,
        remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
        provider: 'github',
        codeExecution: {
          dockerImage: 'ghcr.io/aorwall/moatless-vibe-sandbox-js:latest',
          setupCommands: ['bun install'],
        },
        previewConfig: {
          serverStartCommands: ['bun run dev -- --host --port {PORT}'],
          startupTimeoutSeconds: 60,
        },
      },
    })

    if (createRepoResponse.statusCode !== 200) {
      throw new Error(`Failed to create repository: ${createRepoResponse.body}`)
    }

    const repository = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id
    logger.info({ repositoryId: createdRepositoryId }, 'Test repository created')

    // Create task WITHOUT a message - this prevents iteration creation and auto-start
    // This keeps preview.needsSetup true so setup commands are included
    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: {
        agentType: 'claude-code',
        repositoryId: createdRepositoryId,
        // Note: NOT including message - this prevents iteration creation and auto-start
        // Task will be started manually in Agent tests
      },
    })

    if (createTaskResponse.statusCode !== 200) {
      throw new Error(`Failed to create task: ${createTaskResponse.body}`)
    }

    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId
    logger.info({ taskId: createdTaskId }, 'Test task created')

    // Trigger Docker provisioning by accessing files (this calls ensureWorkspace)
    await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/files?recursive=false`,
    })

    // Wait for Docker workspace to be ready
    const workspaceReady = await waitForWorkspaceReady(workspaceCoordinator, createdTaskId, 180000)
    if (!workspaceReady) {
      throw new Error('Docker workspace failed to become ready within timeout')
    }

    logger.info('Docker workspace is ready')

    // Wait for auto-started preview to be ready
    // The WorkspaceCoordinator auto-starts preview when repository has previewConfig
    const previewReady = await waitForPreviewReady(workspaceCoordinator, createdTaskId, 120000)
    if (!previewReady) {
      throw new Error('Preview server failed to start within timeout')
    }

    logger.info('Preview server is ready')
  }, 300000) // 5 minute timeout for container provisioning

  afterAll(async () => {
    // Stop any running agents
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        await new Promise((resolve) => setTimeout(resolve, 500))
      } catch {
        // Agent might not be running
      }

      // Delete task (triggers container cleanup)
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch {
        // Task might already be deleted
      }
    }

    // Delete repository
    if (createdRepositoryId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${createdRepositoryId}`,
        })
      } catch {
        // Repository might already be deleted
      }
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch {
        // Directory might already be deleted
      }
    }

    // Clean up environment variables
    Reflect.deleteProperty(process.env, 'WORKSPACE_EXECUTION_MODE')
    Reflect.deleteProperty(process.env, 'DATA_DIR')
    Reflect.deleteProperty(process.env, 'SKIP_CLAUDE_DISK_EVENTS')

    await app.close()
  }, 60000)

  describe('Workspace Provisioning', () => {
    it('should have created Docker workspace', async () => {
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)

      expect(snapshot).toBeTruthy()
      expect(snapshot?.executionTarget.mode).toBe('docker')
    })

    it('should report Docker execution target in snapshot', async () => {
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)

      expect(snapshot?.executionTarget.mode).toBe('docker')

      const target = snapshot?.executionTarget as {
        containerName?: string
        containerStatus?: string
        hostPort?: number
      }

      expect(target.containerName).toBeDefined()
      expect(target.containerStatus).toBe('running')
      expect(target.hostPort).toBeGreaterThan(0)
    })

    it('should reach container health endpoint', async () => {
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
      const target = snapshot?.executionTarget as { hostPort?: number }

      expect(target.hostPort).toBeDefined()

      const healthResponse = await fetch(`http://localhost:${target.hostPort}/health`, {
        signal: AbortSignal.timeout(5000),
      })

      expect(healthResponse.ok).toBe(true)

      const healthData = (await healthResponse.json()) as { status: string; mode: string }
      expect(healthData.status).toBe('ok')
      expect(healthData.mode).toBe('sandbox')
    })
  })

  describe('File Operations via Docker', () => {
    it('should list files in Docker workspace', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/files?recursive=false`,
      })

      expect(response.statusCode).toBe(200)

      const data = JSON.parse(response.body)
      expect(data.files).toBeDefined()
      expect(Array.isArray(data.files)).toBe(true)
      expect(data.files.length).toBeGreaterThan(0)

      // hello-world-ui has package.json
      const hasPackageJson = data.files.some(
        (f: { path: string }) => f.path === 'package.json' || f.path.endsWith('/package.json'),
      )
      expect(hasPackageJson).toBe(true)
    })

    it('should read file from Docker workspace', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/files/package.json`,
      })

      expect(response.statusCode).toBe(200)

      const data = JSON.parse(response.body)
      expect(data.content).toBeDefined()
      // hello-world-ui package name
      expect(data.content).toContain('vite-react-typescript-starter')
    })

    it('should write file to Docker workspace', async () => {
      const testContent = `Test file created at ${new Date().toISOString()}`

      // Write file
      const writeResponse = await app.inject({
        method: 'PUT',
        url: `/api/v1/tasks/${createdTaskId}/files/test-docker-write.txt`,
        payload: { content: testContent },
      })

      expect(writeResponse.statusCode).toBe(200)

      // Read back
      const readResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/files/test-docker-write.txt`,
      })

      expect(readResponse.statusCode).toBe(200)

      const data = JSON.parse(readResponse.body)
      expect(data.content).toBe(testContent)
    })
  })

  describe('Git Operations via Docker', () => {
    it('should get git diff from Docker workspace', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/git-diff`,
      })

      expect(response.statusCode).toBe(200)

      const data = JSON.parse(response.body)
      expect(data).toBeDefined()
      // Diff may be empty if no changes have been made
    })

    it('should get git diff after file changes', async () => {
      // First write a file to create changes
      await app.inject({
        method: 'PUT',
        url: `/api/v1/tasks/${createdTaskId}/files/git-test-file.txt`,
        payload: { content: 'Test content for git diff' },
      })

      // Now get git diff - should show the new file
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/git-diff`,
      })

      expect(response.statusCode).toBe(200)

      const data = JSON.parse(response.body)
      expect(data).toBeDefined()
      // Should have changes now
    })
  })

  describe('Preview Server via Docker', () => {
    // Preview is auto-started by WorkspaceCoordinator during workspace provisioning
    // These tests verify the preview lifecycle after auto-start

    it('should have preview server already started (auto-start)', async () => {
      // Verify snapshot shows preview as started
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
      expect(snapshot?.preview.status).toBe('started')
    })

    it('should get preview server status', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/server/status`,
      })

      expect(response.statusCode).toBe(200)

      const data = JSON.parse(response.body)
      expect(data.status).toBe('started')
    })

    it('should stop preview server', async () => {
      const stopResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/server/stop`,
      })

      expect(stopResponse.statusCode).toBe(200)

      const data = JSON.parse(stopResponse.body)
      expect(data.stopped).toBe(true)

      // Wait a moment for state to update
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Verify status
      const statusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/server/status`,
      })

      expect(statusResponse.statusCode).toBe(200)

      const statusData = JSON.parse(statusResponse.body)
      expect(statusData.status).toBe('stopped')
    })

    it('should restart preview server after stop', async () => {
      // Start preview server again
      const startResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/server/start`,
      })

      expect(startResponse.statusCode).toBe(202) // Accepted

      // Wait for preview to be ready
      const previewReady = await waitForPreviewReady(workspaceCoordinator, createdTaskId, 120000)
      expect(previewReady).toBe(true)

      // Verify snapshot
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
      expect(snapshot?.preview.status).toBe('started')
    }, 180000)
  })

  // Agent tests require Claude authentication (ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN)
  describe.skipIf(!hasClaudeAuth())('Agent Execution via Docker', () => {
    it('should execute agent in Docker container and create file', async () => {
      // Start agent with simple task - this returns an SSE stream
      // We just need to verify the request succeeds; agent execution is asynchronous
      const runResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/agent/start`,
        payload: {
          message:
            'Create a file called hello-from-docker.txt with the content "Hello from Docker container!"',
        },
      })

      expect(runResponse.statusCode).toBe(200)

      // Wait for agent to become idle (poll for completion)
      const agentIdle = await waitForAgentIdle(workspaceCoordinator, createdTaskId, 90000)
      expect(agentIdle).toBe(true)

      // Verify file was created
      const readResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/files/hello-from-docker.txt`,
      })

      expect(readResponse.statusCode).toBe(200)

      const fileData = JSON.parse(readResponse.body)
      expect(fileData.content).toContain('Hello from Docker')

      logger.info({ fileContent: fileData.content }, 'Agent execution completed successfully')
    }, 120000)

    it('should show correct task status after agent completion', async () => {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}`,
      })

      expect(response.statusCode).toBe(200)

      const task: Task = JSON.parse(response.body)
      expect(task.taskId).toBe(createdTaskId)
      // Task exists and agent execution didn't break it
    })
  })
})

// Provide skip messages when tests cannot run
if (!canRunTests) {
  describe('Docker Workflow Integration Test', () => {
    if (!dockerAvailable) {
      it.skip('Tests skipped - Docker not available', () => {
        // Skipped test - provides visibility in test output
      })
    } else if (!platformCompatible) {
      it.skip('Tests skipped - Platform not compatible (requires x64 or arm64)', () => {
        // Skipped test - provides visibility in test output
      })
    } else if (!hasAuth) {
      it.skip('Tests skipped - No Claude authentication (ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN required)', () => {
        // Skipped test - provides visibility in test output
      })
    }
  })
}
