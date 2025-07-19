import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  RepositoryConfig,
  ServerStatus,
} from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

/**
 * Test for Preview Environment Error Handling
 *
 * This test verifies that when server startup fails:
 * - The service returns a ServerStatus with running: false and error details
 * - Full error messages and logs are captured and returned
 * - The system remains in a stable state (no crashes or unhandled errors)
 */
describe('Preview Environment Error Handling', () => {
  let app: any
  let tempDir: string
  let createdRepositoryIds: string[] = []
  let createdTaskIds: string[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test environment
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'preview-error-test-'))

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    app = await build()
  })

  afterEach(async () => {
    // Cleanup tasks
    for (const taskId of createdTaskIds) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${taskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
      }
    }

    // Cleanup repositories
    for (const repoId of createdRepositoryIds) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${repoId}`,
        })
      } catch (_e) {
        // Repository might already be deleted
      }
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }

    // Reset arrays
    createdRepositoryIds = []
    createdTaskIds = []
  })

  it('should return error state when server command fails', async () => {
    // Create a repository with an invalid server command
    const repository = await createRepository('Invalid Command Test')
    const task = await createTask(repository.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Attempt to start the server with invalid command
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })

    // The API should succeed (not throw) but return error state
    expect(startResponse.statusCode).toBe(200)
    const serverStatus: ServerStatus = JSON.parse(startResponse.body)

    // Verify the server is not running
    expect(serverStatus.running).toBe(false)
    expect(serverStatus.configured).toBe(true)

    // Verify error details are captured
    expect(serverStatus.error).toBeDefined()
    expect(serverStatus.error).toContain('Failed to start server')

    // Should have some error message about the invalid command
    expect(serverStatus.error).toMatch(/command.*(not found|doesn't exist|invalid)/i)

    // Logs should be available (might be empty but field should exist)
    expect(serverStatus.logs).toBeDefined()

    console.log('Server error message:', serverStatus.error)
    console.log('Server logs:', serverStatus.logs)
  }, 30000)

  it('should return error state when server command exists but fails', async () => {
    // Create a script that exists but fails during execution
    const failingScript = `
#!/bin/bash
echo "Starting server that will fail..."
echo "Error: Missing configuration file" >&2
exit 1
`
    const scriptPath = path.join(tempDir, 'failing-server.sh')
    await fs.writeFile(scriptPath, failingScript)
    await fs.chmod(scriptPath, 0o755)

    const repository = await createRepository('Failing Server Test')
    const task = await createTask(repository.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Attempt to start the failing server
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })

    // The API should succeed but return error state
    expect(startResponse.statusCode).toBe(200)
    const serverStatus: ServerStatus = JSON.parse(startResponse.body)

    // Verify the server is not running
    expect(serverStatus.running).toBe(false)
    expect(serverStatus.configured).toBe(true)

    // Verify error details are captured
    expect(serverStatus.error).toBeDefined()
    expect(serverStatus.error).toContain('Failed to start server')

    // Verify logs contain the error output
    expect(serverStatus.logs).toBeDefined()
    // The logs should contain our error message from stderr
    expect(serverStatus.logs).toContain('Missing configuration file')

    console.log('Failing server error:', serverStatus.error)
    console.log('Failing server logs:', serverStatus.logs)
  }, 30000)

  it('should handle npm/bun command not found errors', async () => {
    // Simulate the specific error from the issue: "start:frontend" not found
    const packageJsonContent = `{
  "name": "test-app",
  "version": "1.0.0",
  "scripts": {
    "start": "echo 'Wrong script name'",
    "start:production": "echo 'Production script'"
  }
}`

    await fs.writeFile(path.join(tempDir, 'package.json'), packageJsonContent)

    const repository = await createRepository('NPM Command Error Test')
    const task = await createTask(repository.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Attempt to start with non-existent npm script
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })

    // The API should succeed but return error state
    expect(startResponse.statusCode).toBe(200)
    const serverStatus: ServerStatus = JSON.parse(startResponse.body)

    // Verify the server is not running
    expect(serverStatus.running).toBe(false)
    expect(serverStatus.configured).toBe(true)

    // Verify error details capture the npm/bun error
    expect(serverStatus.error).toBeDefined()
    expect(serverStatus.error).toContain('Failed to start server')

    // Should contain information about the missing script
    expect(serverStatus.error).toMatch(/start:frontend.*(not found|doesn't exist)/i)

    // Logs should contain bun error details
    expect(serverStatus.logs).toBeDefined()

    console.log('NPM error message:', serverStatus.error)
    console.log('NPM error logs:', serverStatus.logs)
  }, 30000)

  it('should return correct server status after failed startup', async () => {
    // Test that subsequent status checks work correctly after a failed startup
    const repository = await createRepository('Status After Failure Test')
    const task = await createTask(repository.id)

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Attempt to start server (should fail)
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${task.taskId}/server/start`,
    })
    expect(startResponse.statusCode).toBe(200)

    const startResult: ServerStatus = JSON.parse(startResponse.body)
    expect(startResult.running).toBe(false)

    // Check server status via GET endpoint
    const statusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${task.taskId}/server/status`,
    })
    expect(statusResponse.statusCode).toBe(200)

    const status: ServerStatus = JSON.parse(statusResponse.body)
    expect(status.running).toBe(false)
    expect(status.configured).toBe(true)

    // Error details should persist in status checks
    if (startResult.error) {
      expect(status.error).toBeDefined()
    }
  }, 30000)

  // Helper functions
  async function createRepository(name: string): Promise<RepositoryConfig> {
    const request: CreateRepositoryRequest = {
      name,
      description: 'Test repository for error handling',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        setupCommands: ['echo "Environment ready"'],
      },
    }

    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: request,
    })

    expect(response.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(response.body)
    createdRepositoryIds.push(repository.id)
    return repository
  }

  async function createTask(repositoryId: string): Promise<{ taskId: string }> {
    const request: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId,
      start: false, // Don't start agent
    }

    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: request,
    })

    expect(response.statusCode).toBe(200)
    const result = JSON.parse(response.body)
    createdTaskIds.push(result.taskId)
    return result
  }
})

async function setupTempGitRepo(tempDir: string): Promise<void> {
  const { spawn } = await import('node:child_process')

  // Initialize git repository
  await runCommand('git', ['init'], tempDir)
  await runCommand('git', ['config', 'user.name', 'Test User'], tempDir)
  await runCommand('git', ['config', 'user.email', 'test@example.com'], tempDir)

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# Test Repository\n\nThis is a test repository for error handling testing.\n',
  )

  // Add and commit initial file
  await runCommand('git', ['add', 'README.md'], tempDir)
  await runCommand('git', ['commit', '-m', 'Initial commit'], tempDir)

  async function runCommand(command: string, args: string[], cwd: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const child = spawn(command, args, {
        cwd,
        stdio: 'pipe',
        env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
      })

      let stderr = ''
      child.stderr?.on('data', (data: Buffer) => {
        stderr += data.toString()
      })

      child.on('close', (code: number) => {
        if (code !== 0) {
          reject(
            new Error(`Command ${command} ${args.join(' ')} failed with code ${code}: ${stderr}`),
          )
        } else {
          resolve()
        }
      })

      child.on('error', reject)
    })
  }
}
