import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  RepositoryConfig,
  Task,
} from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TestAgent } from '../../agents/test-agent.js'
import { build } from '../../test/helper.js'

/**
 * Agent Model Parameter Integration Test
 *
 * This test verifies that the model parameter is correctly passed through
 * all layers of the system:
 * 1. From API request to agent service
 * 2. Through runOptions in agent execution
 * 3. To the actual agent implementation
 * 4. Used by the agent for processing
 */

// Mock the local environment to use TestAgent
vi.mock('../../environments/local-environment.js', async () => {
  const actual = await vi.importActual('../../environments/local-environment.js')
  return {
    ...actual,
    LocalEnvironment: class extends (actual as any).LocalEnvironment {
      async createAgent(_agentType: string, options: any) {
        // Create TestAgent instead of ClaudeCodeAgent
        const agent = new TestAgent('test-task-id', options)
        // Store reference for tests to verify
        ;(global as any).__lastCreatedTestAgent = agent
        return agent
      }
    },
  }
})

async function setupTempGitRepo(tempDir: string): Promise<void> {
  // Initialize git repository
  await runCommand('git', ['init'], tempDir)
  await runCommand('git', ['config', 'user.name', 'Test User'], tempDir)
  await runCommand('git', ['config', 'user.email', 'test@example.com'], tempDir)

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# Test Repository\n\nThis is a test repository for model parameter testing.\n',
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

describe('Agent Model Parameter Test', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string

  beforeEach(async () => {
    // Clear any previous test agent
    delete (global as any).__lastCreatedTestAgent

    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-model-test-'))

    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

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
    delete process.env['SKIP_CLAUDE_DISK_EVENTS']
    delete (global as any).__lastCreatedTestAgent

    // Wait for any background processes to finish and clean up properly
    await new Promise((resolve) => setTimeout(resolve, 1000))

    await app.close()
    vi.clearAllMocks()
  })

  it('should pass model parameter through all layers when creating task with model', async () => {
    // Step 1: Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'model-parameter-test-repository',
      description: 'Test repository for model parameter verification',
      path: tempDir,
      useGitWorktrees: false,
      cloneOnNewTask: false,
      codeExecution: {
        type: 'local',
        setupCommands: ['echo "Repository ready for model parameter test"'],
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    if (createRepoResponse.statusCode !== 200) {
      console.error('Failed to create repository:', createRepoResponse.body)
    }
    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Step 2: Create and start a task with specific model
    const testModel = 'claude-3-opus-20240229'
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Create a simple test.txt file',
      start: true,
      model: testModel,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    if (createTaskResponse.statusCode !== 200) {
      console.error('Failed to create task:', createTaskResponse.body)
    }
    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId
    expect(taskCreationResult.status).toBe('open')

    // Wait for the agent to start processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 3: Verify the model was passed to the test agent
    const testAgent = (global as any).__lastCreatedTestAgent as TestAgent
    expect(testAgent).toBeDefined()
    expect(testAgent.capturedOptions.model).toBe(testModel)

    // Also verify it would be used in runOptions
    expect(testAgent.getEffectiveModel()).toBe(testModel)

    // Step 4: Verify task has the model stored
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(getTaskResponse.statusCode).toBe(200)
    const task: Task = JSON.parse(getTaskResponse.body)

    // Check that the latest iteration has the model
    const latestIteration = task.iterations[task.iterations.length - 1]
    expect(latestIteration).toBeDefined()
    if (!latestIteration) throw new Error('Expected latest iteration')
    expect(latestIteration.model).toBe(testModel)
  }, 30000)

  it('should pass model parameter when sending a message with model', async () => {
    // Step 1: Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'model-parameter-message-test-repository',
      path: tempDir,
      useGitWorktrees: false,
      cloneOnNewTask: false,
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

    // Step 2: Create a task WITHOUT starting it
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

    // Clear any previous test agent
    delete (global as any).__lastCreatedTestAgent

    // Step 3: Send a message with a specific model
    const testModel = 'claude-3-sonnet-20240320'
    const sendMessageResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
      payload: {
        message: 'Create a file called model_test.py',
        model: testModel,
      },
    })

    expect(sendMessageResponse.statusCode).toBe(200)

    // Wait for agent to process
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 4: Verify the model was passed to the test agent
    const testAgent = (global as any).__lastCreatedTestAgent as TestAgent
    expect(testAgent).toBeDefined()
    // The agent should have received the model in runOptions
    expect(testAgent.capturedRunOptions.model).toBe(testModel)
    expect(testAgent.getEffectiveModel(testAgent.capturedRunOptions)).toBe(testModel)

    // Step 5: Verify the iteration has the correct model
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    const task: Task = JSON.parse(getTaskResponse.body)
    const latestIteration = task.iterations[task.iterations.length - 1]
    expect(latestIteration).toBeDefined()
    if (!latestIteration) throw new Error('Expected latest iteration')
    expect(latestIteration.model).toBe(testModel)
  }, 30000)

  it('should use default model when no model is specified', async () => {
    // Step 1: Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'default-model-test-repository',
      path: tempDir,
      useGitWorktrees: false,
      cloneOnNewTask: false,
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

    // Step 2: Create and start a task WITHOUT specifying model
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Create a simple default_test.txt file',
      start: true,
      // No model specified
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId

    // Wait for the agent to start processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 3: Verify the default model was used
    const testAgent = (global as any).__lastCreatedTestAgent as TestAgent
    expect(testAgent).toBeDefined()
    // Should use the default model when none specified
    expect(testAgent.capturedOptions.model).toBeUndefined()
    expect(testAgent.getEffectiveModel()).toBe('default-test-model')
  }, 30000)
})
