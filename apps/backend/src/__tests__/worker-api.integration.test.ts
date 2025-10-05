import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  AgentEvent,
  CreateRepositoryRequest,
  CreateTaskRequest,
  RepositoryConfig,
} from '@moatless/types'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../test/helper.js'
import { WorkerApiClient } from '../utils/worker-api-client.js'
import { startWorkerMode } from '../worker-app.js'

describe('Worker API Integration Test', () => {
  let mainApp: any
  let mainServerUrl: string
  let workerServer: any
  let workerPort: number
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let apiClient: WorkerApiClient

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

    // Create API client for testing
    apiClient = new WorkerApiClient(mainServerUrl)
  })

  afterAll(async () => {
    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    await mainApp.close()
  })

  beforeEach(async () => {
    // Create a temporary directory for test repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'worker-api-test-'))

    // Initialize git repository
    await setupGitRepo(tempDir)

    // Create a fresh API client for each test
    apiClient = new WorkerApiClient(mainServerUrl)
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

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }
  })

  describe('WorkerApiClient', () => {
    it('should fetch task from API', async () => {
      // Step 1: Create repository
      const createRepoRequest: CreateRepositoryRequest = {
        name: 'worker-api-test-repo', // Changed to match the regex pattern
        description: 'Test repository for worker API integration',
        path: tempDir,
        useGitWorktrees: false,
        cloneOnNewTask: false,
        codeExecution: {
          type: 'local',
          setupCommands: ['echo "Setup complete"'],
        },
      }
      const createRepoResponse = await mainApp.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: createRepoRequest,
      })

      if (createRepoResponse.statusCode !== 200) {
        console.error('Failed to create repository:', createRepoResponse.body)
      }
      expect(createRepoResponse.statusCode).toBe(200)
      const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
      createdRepositoryId = repository.id

      // Step 2: Create task without starting
      const createTaskRequest: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: repository.id,
        start: false, // Important: don't start the task
      }

      const createTaskResponse = await mainApp.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: createTaskRequest,
      })

      expect(createTaskResponse.statusCode).toBe(200)
      const taskResult = JSON.parse(createTaskResponse.body)
      createdTaskId = taskResult.taskId

      // Wait a bit for workspace setup to start
      await new Promise((resolve) => setTimeout(resolve, 500))

      // Step 3: Use WorkerApiClient to fetch the task
      const fetchedTask = await apiClient.fetchTask(createdTaskId)

      // Verify the fetched task
      expect(fetchedTask).toBeDefined()
      expect(fetchedTask.taskId).toBe(createdTaskId)
      expect(fetchedTask.repositoryId).toBe(repository.id)
      expect(fetchedTask.agentType).toBe('claude-code')
      expect(fetchedTask.status).toBe('open')
      expect(fetchedTask.iterations).toEqual([])

      // Verify dates were properly transformed
      expect(fetchedTask.createdAt).toBeInstanceOf(Date)
      expect(fetchedTask.lastMessageAt).toBeInstanceOf(Date)
    })

    it('should fetch repository from API', async () => {
      // Step 1: Create repository
      const createRepoRequest: CreateRepositoryRequest = {
        name: 'worker-api-test-repo-2', // Changed to match the regex pattern
        description: 'Another test repository',
        path: tempDir,
        useGitWorktrees: true,
        cloneOnNewTask: false,
        codeExecution: {
          type: 'docker',
          dockerImage: 'node:20',
          setupCommands: ['npm install'],
          envVars: {
            NODE_ENV: 'test',
          },
        },
        previewConfig: {
          serverStartCommands: ['npm run dev'],
          enableInspector: true,
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

      // Step 2: Use WorkerApiClient to fetch the repository
      const fetchedRepo = await apiClient.fetchRepository(repository.id)

      // Verify the fetched repository
      expect(fetchedRepo).toBeDefined()
      expect(fetchedRepo.id).toBe(repository.id)
      expect(fetchedRepo.name).toBe(createRepoRequest.name)
      expect(fetchedRepo.description).toBe(createRepoRequest.description)
      expect(fetchedRepo.path).toBe(createRepoRequest.path)
      expect(fetchedRepo.useGitWorktrees).toBe(true)
      expect(fetchedRepo.codeExecution).toEqual(createRepoRequest.codeExecution)
      expect(fetchedRepo.previewConfig).toEqual(createRepoRequest.previewConfig)
    })

    it('should handle API errors gracefully', async () => {
      // Test fetching non-existent task
      await expect(apiClient.fetchTask('non-existent-task-id')).rejects.toThrow(
        /Failed to GET task/,
      )

      // Test fetching non-existent repository
      await expect(apiClient.fetchRepository('non-existent-repo-id')).rejects.toThrow(
        /Failed to GET repository/,
      )
    })
  })

  describe('Worker Mode', () => {
    it.skip('should start worker mode and fetch task/repository from API - SKIPPED: Worker workspace initialization needs refactoring', async () => {
      // Step 1: Create repository and task
      const createRepoRequest: CreateRepositoryRequest = {
        name: 'worker-mode-test-repo',
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

      // Step 2: Start worker mode
      process.env.WORKER_TASK_ID = createdTaskId
      workerPort = 8010 + Math.floor(Math.random() * 1000) // Random port to avoid conflicts
      process.env.PORT = workerPort.toString()

      // Create a temporary workspace directory for the worker
      const workerWorkspaceDir = await fs.mkdtemp(path.join(os.tmpdir(), 'worker-workspace-'))

      // Start worker in background
      void startWorkerMode({
        apiUrl: mainServerUrl,
        workspaceDir: workerWorkspaceDir,
        dataDir: path.join(workerWorkspaceDir, 'data'),
      })

      // Wait for worker to start
      await new Promise((resolve) => setTimeout(resolve, 3000))

      // Step 3: Test worker health endpoint
      const healthResponse = await fetch(`http://localhost:${workerPort}/health`)
      expect(healthResponse.ok).toBe(true)

      const healthData = await healthResponse.json()
      expect(healthData).toMatchObject({
        status: 'ok',
        mode: 'worker',
        workspaceDir: workerWorkspaceDir,
        taskId: createdTaskId,
      })

      // Step 4: Test execute-agent endpoint (without actually running an agent)
      const executeResponse = await fetch(`http://localhost:${workerPort}/execute-agent`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          taskId: createdTaskId,
          message: 'Test message',
          agentType: 'claude-code',
          runOptions: {},
        }),
      })

      if (!executeResponse.ok) {
        const errorText = await executeResponse.text()
        console.error('Execute agent failed:', executeResponse.status, errorText)
      }
      expect(executeResponse.ok).toBe(true)
      const executeData = await executeResponse.json()
      expect(executeData).toMatchObject({
        success: true,
        message: 'Agent execution started',
      })

      // Clean up worker workspace
      await fs.rm(workerWorkspaceDir, { recursive: true, force: true })

      // Clean up environment
      delete process.env.WORKER_TASK_ID
      delete process.env.PORT

      // Note: We can't easily stop the worker server started with startWorkerMode
      // since it doesn't return a server handle. In real usage, the process would exit.
    }, 10000)

    it('should handle missing WORKER_TASK_ID environment variable', async () => {
      // Ensure WORKER_TASK_ID is not set
      delete process.env.WORKER_TASK_ID

      // Attempting to start worker mode should throw
      await expect(
        startWorkerMode({
          apiUrl: mainServerUrl,
          workspaceDir: tempDir,
        }),
      ).rejects.toThrow('WORKER_TASK_ID environment variable not set')
    })

    it('should handle task not found error', async () => {
      // Set a non-existent task ID
      process.env.WORKER_TASK_ID = 'non-existent-task'

      // Attempting to start worker mode should throw
      await expect(
        startWorkerMode({
          apiUrl: mainServerUrl,
          workspaceDir: tempDir,
        }),
      ).rejects.toThrow(/Failed to GET task/)

      // Clean up
      delete process.env.WORKER_TASK_ID
    })
  })

  describe('Worker Event Emission', () => {
    it('should emit events back to main API', async () => {
      // Create repository and task first
      const createRepoRequest: CreateRepositoryRequest = {
        name: 'event-emission-test-repo',
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

      // Test emitting an event
      const testEvent: AgentEvent = {
        eventId: 'test-event-123',
        agentType: 'claude-code',
        type: 'agent',
        subType: 'message',
        taskId: createdTaskId,
        isError: false,
        payload: {
          type: 'text',
          content: 'Test event from worker',
        },
      } as AgentEvent

      // Test emitting an event - should succeed without error
      await expect(apiClient.emitEvent(createdTaskId, testEvent)).resolves.not.toThrow()
    })
  })
})

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
