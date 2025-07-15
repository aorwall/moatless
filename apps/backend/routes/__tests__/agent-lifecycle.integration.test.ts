import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  RepositoryConfig,
  Task,
} from '@moatless/types/types/index'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

/**
 * Agent Lifecycle Integration Test
 *
 * This test verifies that the server can:
 * 1. Start an agent successfully
 * 2. Stop the agent gracefully
 * 3. Continue running and handle new requests after agent shutdown
 *
 * This ensures proper agent lifecycle management and server resilience.
 */

async function setupTempGitRepo(tempDir: string): Promise<void> {
  // Initialize git repository
  await runCommand('git', ['init'], tempDir)
  await runCommand('git', ['config', 'user.name', 'Test User'], tempDir)
  await runCommand('git', ['config', 'user.email', 'test@example.com'], tempDir)

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# Test Repository\n\nThis is a test repository for agent lifecycle testing.\n',
  )

  // Add and commit initial file
  await runCommand('git', ['add', 'README.md'], tempDir)
  await runCommand('git', ['commit', '-m', 'Initial commit'], tempDir)
}

async function runCommand(command: string, args: string[], cwd: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const { spawn } = require('node:child_process')
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

describe('Agent Lifecycle Test', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-agent-lifecycle-'))

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    app = await build()

    // Start the server on a random available port
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
  })

  afterEach(async () => {
    // Cleanup task if it exists
    if (createdTaskId) {
      try {
        // First try to stop the agent
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        // Give it a moment to clean up
        await new Promise((resolve) => setTimeout(resolve, 100))

        // Then delete the task
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted or not exist
      }
    }

    // Cleanup repository if it exists
    if (createdRepositoryId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${createdRepositoryId}`,
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

    // Clean up test environment variable
    delete process.env.SKIP_CLAUDE_DISK_EVENTS

    // Wait longer for any background processes to finish and clean up properly
    await new Promise((resolve) => setTimeout(resolve, 5000))

    await app.close()
  })

  it('should start an agent, stop it, and verify server continues running', async () => {
    // Step 1: Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Agent Lifecycle Test Repository',
      description: 'Test repository for agent lifecycle verification',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        setupCommands: 'echo "Repository ready for agent lifecycle test"',
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Step 2: Create and start a task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message:
        'Create a file called lifecycle_test.py with a simple Python function that adds two numbers',
      start: true,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId
    expect(taskCreationResult.status).toBe('open')

    // Wait for the agent to start processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 3: Verify agent is running
    let agentStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/agent-status`,
    })

    expect(agentStatusResponse.statusCode).toBe(200)
    let agentStatus = JSON.parse(agentStatusResponse.body)

    // Wait for agent to actually start running
    let attempts = 0
    while (agentStatus.status !== 'running' && attempts < 10) {
      await new Promise((resolve) => setTimeout(resolve, 500))
      agentStatusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/agent-status`,
      })
      agentStatus = JSON.parse(agentStatusResponse.body)
      attempts++
    }

    expect(agentStatus.status).toBe('running')
    expect(agentStatus.sessionId).toBeTruthy()
    console.log('Agent is running with session:', agentStatus.sessionId)

    // Step 4: Stop the agent
    const stopResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/stop`,
    })

    expect(stopResponse.statusCode).toBe(200)
    const stopResult = JSON.parse(stopResponse.body)
    expect(stopResult.status).toBe('stopped')
    expect(stopResult.taskId).toBe(createdTaskId)
    console.log('Agent stopped successfully')

    // Wait longer for the agent to fully stop and clean up
    console.log('Waiting for agent to fully clean up...')
    await new Promise((resolve) => setTimeout(resolve, 5000))

    // Step 5: Verify agent is no longer running
    agentStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/agent-status`,
    })

    expect(agentStatusResponse.statusCode).toBe(200)
    agentStatus = JSON.parse(agentStatusResponse.body)
    expect(agentStatus.status).toBe('idle')
    console.log('Agent status after stop:', agentStatus.status)

    // Step 6: Verify server is still responsive by making various API calls
    console.log('Verifying server is still responsive...')

    // Test 1: List repositories
    const listRepoResponse = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories',
    })
    expect(listRepoResponse.statusCode).toBe(200)
    const repoData = JSON.parse(listRepoResponse.body)
    expect(repoData).toHaveProperty('repositories')
    expect(Array.isArray(repoData.repositories)).toBe(true)
    console.log('✅ Can list repositories')

    // Test 2: Get task details
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })
    expect(getTaskResponse.statusCode).toBe(200)
    const task: Task = JSON.parse(getTaskResponse.body)
    expect(task.taskId).toBe(createdTaskId)
    console.log('✅ Can get task details')

    // Test 3: Get task events
    const getEventsResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/events`,
    })
    expect(getEventsResponse.statusCode).toBe(200)
    const { events } = JSON.parse(getEventsResponse.body)
    expect(Array.isArray(events)).toBe(true)
    console.log('✅ Can get task events')

    // Test 4: Create a new task to verify we can start new agents
    const createNewTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Create a simple hello.txt file',
      start: false, // Don't start it, just create
    }

    const createNewTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createNewTaskRequest,
    })

    expect(createNewTaskResponse.statusCode).toBe(200)
    const newTaskResult = JSON.parse(createNewTaskResponse.body)
    const newTaskId = newTaskResult.taskId
    expect(newTaskResult.status).toBe('open')
    console.log('✅ Can create new tasks')

    // Cleanup the second task
    await app.inject({
      method: 'DELETE',
      url: `/api/v1/tasks/${newTaskId}`,
    })

    // Test 5: Health check endpoint (if available)
    const healthResponse = await app.inject({
      method: 'GET',
      url: '/health',
    })
    // Health endpoint might not exist, but server should still respond
    expect([200, 404]).toContain(healthResponse.statusCode)
    console.log('✅ Server is responsive to health check')

    console.log('\n✅ All tests passed! Server successfully handled agent lifecycle:')
    console.log('  - Started agent')
    console.log('  - Stopped agent gracefully')
    console.log('  - Continued serving requests')
    console.log('  - Can create new tasks')
  }, 60000) // 60 second timeout to accommodate cleanup wait times

  it('should handle multiple agent start/stop cycles', async () => {
    // Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Multiple Agent Cycles Test',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Create a task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      start: false,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId

    // Wait for workspace to be ready
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Perform multiple start/stop cycles
    const cycles = 3
    for (let i = 0; i < cycles; i++) {
      console.log(`\nCycle ${i + 1}/${cycles}:`)

      // Send a message to start the agent
      const sendMessageResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/messages`,
        payload: {
          message: `Create a file cycle_${i + 1}.txt with content "Cycle ${i + 1}"`,
        },
      })
      expect(sendMessageResponse.statusCode).toBe(200)
      console.log('  - Started agent with message')

      // Wait for agent to start
      await new Promise((resolve) => setTimeout(resolve, 2000))

      // Verify agent is running
      const agentStatusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/agent-status`,
      })
      const agentStatus = JSON.parse(agentStatusResponse.body)
      console.log(`  - Agent status: ${agentStatus.status}`)

      // Stop the agent
      const stopResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${createdTaskId}/stop`,
      })
      expect(stopResponse.statusCode).toBe(200)
      console.log('  - Stopped agent')

      // Wait longer before next cycle to ensure clean shutdown
      await new Promise((resolve) => setTimeout(resolve, 3000))

      // Verify server is still responsive
      const taskResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}`,
      })
      expect(taskResponse.statusCode).toBe(200)
      console.log('  - Server still responsive')
    }

    console.log('\n✅ Successfully completed multiple agent start/stop cycles')
  }, 90000) // 90 second timeout to accommodate multiple cycles with cleanup
})
