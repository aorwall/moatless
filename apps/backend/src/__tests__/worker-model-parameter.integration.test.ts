import { promises as fs } from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import type { CreateRepositoryRequest, CreateTaskRequest, RepositoryConfig } from '@moatless/types'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { TestAgent } from '../agents/test-agent.js'
import { build } from '../test/helper.js'
import { startWorkerMode } from '../worker-app.js'

// Mock the environment to use TestAgent
vi.mock('../environments/local-environment.js', async () => {
  const actual = await vi.importActual('../environments/local-environment.js')
  return {
    ...actual,
    LocalEnvironment: class extends (actual as any).LocalEnvironment {
      async createAgent(_agentType: string, options: any) {
        // Create TestAgent instead of ClaudeCodeAgent
        const agent = new TestAgent('worker-test-task', options)
        // Store reference for tests to verify
        ;(global as any).__lastWorkerTestAgent = agent
        return agent
      }
    },
  }
})

async function getAvailablePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, () => {
      const address = server.address()
      if (!address || typeof address === 'string') {
        reject(new Error('Failed to get port'))
        return
      }
      const port = address.port
      server.close(() => resolve(port))
    })
    server.on('error', reject)
  })
}

async function setupGitRepo(dir: string): Promise<void> {
  const { spawn } = require('node:child_process')

  const runGitCommand = (args: string[]) => {
    return new Promise<void>((resolve, reject) => {
      const child = spawn('git', args, {
        cwd: dir,
        stdio: 'pipe',
        env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
      })

      child.on('close', (code: number) => {
        if (code !== 0) {
          reject(new Error(`Git command failed: git ${args.join(' ')}`))
        } else {
          resolve()
        }
      })

      child.on('error', reject)
    })
  }

  await runGitCommand(['init'])
  await runGitCommand(['config', 'user.name', 'Test User'])
  await runGitCommand(['config', 'user.email', 'test@example.com'])

  // Create initial file
  await fs.writeFile(path.join(dir, 'README.md'), '# Test Repository\n')
  await runGitCommand(['add', 'README.md'])
  await runGitCommand(['commit', '-m', 'Initial commit'])
}

describe('Worker Model Parameter Integration Test', () => {
  let mainApp: any
  let mainServerUrl: string
  let workerServer: any
  let workerPort: number
  let tempDir: string
  let workerWorkspaceDir: string
  let createdRepositoryId: string
  let createdTaskId: string

  beforeAll(async () => {
    // Set up test environment
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    // Start the main API server
    mainApp = await build()
    await mainApp.listen({ port: 0, host: '127.0.0.1' })

    // Get the server address after it's fully listening
    const address = mainApp.server.address()
    if (!address || typeof address === 'string') {
      throw new Error('Unable to get server address')
    }
    mainServerUrl = `http://127.0.0.1:${address.port}`

    console.log('Main server started at:', mainServerUrl)
  })

  afterAll(async () => {
    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    await mainApp.close()
  })

  beforeEach(async () => {
    // Clear any previous test agent
    delete (global as any).__lastWorkerTestAgent

    // Create a temporary directory for test repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'worker-model-test-'))

    // Initialize git repository
    await setupGitRepo(tempDir)

    // Create worker workspace directory
    workerWorkspaceDir = await fs.mkdtemp(path.join(os.tmpdir(), 'worker-workspace-'))
  })

  afterEach(async () => {
    // Stop worker if running
    if (workerServer) {
      await workerServer.close()
      workerServer = null
    }

    // Cleanup task
    if (createdTaskId) {
      try {
        // Stop the agent first
        await mainApp.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        await new Promise((resolve) => setTimeout(resolve, 100))

        await mainApp.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
      }
      createdTaskId = ''
    }

    // Cleanup repository
    if (createdRepositoryId) {
      try {
        await mainApp.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${createdRepositoryId}`,
        })
      } catch (_e) {
        // Repository might already be deleted
      }
      createdRepositoryId = ''
    }

    // Clean up temp directories
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }
    if (workerWorkspaceDir) {
      try {
        await fs.rm(workerWorkspaceDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }

    // Clear test agent reference
    delete (global as any).__lastWorkerTestAgent
  })

  it('should pass model parameter through worker execution', async () => {
    // Step 1: Create repository
    const createRepoRequest: CreateRepositoryRequest = {
      name: 'worker-model-test-repo',
      description: 'Test repository for model parameter verification in worker',
      path: tempDir,
      useGitWorktrees: false,
      cloneOnNewTask: false,
      codeExecution: {
        type: 'local',
      },
    }

    const createRepoResponse = await mainApp.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepoRequest,
    })

    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Step 2: Create task without starting
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      start: false,
    }

    const createTaskResponse = await mainApp.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Step 3: Start worker
    workerPort = await getAvailablePort()

    // Set worker mode environment variables
    process.env.WORKER_MODE = 'true'
    process.env.WORKER_TASK_ID = createdTaskId

    workerServer = await startWorkerMode({
      mainApiUrl: mainServerUrl,
      workspaceDir: workerWorkspaceDir,
    } as any)

    const workerUrl = `http://127.0.0.1:${workerPort}`
    console.log(`Worker started at: ${workerUrl}`)

    // Step 4: Send message with specific model through main API
    const testModel = 'claude-3-opus-20240229'
    const sendMessageResponse = await mainApp.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
      payload: {
        message: 'Create a test file with model parameter',
        model: testModel,
      },
    })

    expect(sendMessageResponse.statusCode).toBe(200)

    // Wait for worker to process the request
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Step 5: Verify the model was passed to the agent in worker
    const testAgent = (global as any).__lastWorkerTestAgent as TestAgent
    expect(testAgent).toBeDefined()
    // The model should be passed through runOptions in worker mode
    expect(testAgent.capturedRunOptions.model).toBe(testModel)
    expect(testAgent.getEffectiveModel(testAgent.capturedRunOptions)).toBe(testModel)

    // Clean up environment variables
    delete process.env.WORKER_MODE
    delete process.env.WORKER_TASK_ID
  }, 30000)

  it('should handle model parameter in direct worker API call', async () => {
    // Step 1: Create repository and task (same as above)
    const createRepoRequest: CreateRepositoryRequest = {
      name: 'worker-direct-model-test',
      path: tempDir,
      useGitWorktrees: false,
      cloneOnNewTask: false,
      codeExecution: {
        type: 'local',
      },
    }

    const createRepoResponse = await mainApp.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepoRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      start: false,
    }

    const createTaskResponse = await mainApp.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId

    // Wait for workspace setup
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Step 2: Start worker
    workerPort = await getAvailablePort()

    process.env.WORKER_MODE = 'true'
    process.env.WORKER_TASK_ID = createdTaskId

    workerServer = await startWorkerMode({
      mainApiUrl: mainServerUrl,
      workspaceDir: workerWorkspaceDir,
    } as any)

    // Clear any previous test agent
    delete (global as any).__lastWorkerTestAgent

    // Step 3: Call worker API directly with model parameter
    const testModel = 'claude-3-sonnet-20240320'
    const workerResponse = await fetch(`http://127.0.0.1:${workerPort}/execute-agent`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        taskId: createdTaskId,
        message: 'Test direct worker call',
        agentType: 'claude-code',
        model: testModel,
        runOptions: {
          workspaceDir: '/repo',
        },
      }),
    })

    expect(workerResponse.ok).toBe(true)
    const result = (await workerResponse.json()) as { status: string }
    expect(result.status).toBe('started')

    // Wait for processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 4: Verify the model was passed correctly
    const testAgent = (global as any).__lastWorkerTestAgent as TestAgent
    expect(testAgent).toBeDefined()
    // In direct worker call, model is passed separately and should be in options
    expect(testAgent.capturedOptions.model).toBe(testModel)
    expect(testAgent.getEffectiveModel()).toBe(testModel)

    // Clean up environment variables
    delete process.env.WORKER_MODE
    delete process.env.WORKER_TASK_ID
  }, 30000)
})
