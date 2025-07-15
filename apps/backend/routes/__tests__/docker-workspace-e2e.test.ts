import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  Event,
  RepositoryConfig,
  Task,
} from '@moatless/types'
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'
import { logger } from '../../utils/logger.js'

const execAsync = promisify(exec)

/**
 * E2E Test for Docker Workspace with ClaudeCodeAgent
 *
 * This test validates the complete Docker workspace workflow including:
 * - Repository creation with Docker workspace type
 * - Docker container creation and management
 * - Task creation and agent execution inside container
 * - Worker mode communication between host and container
 * - File operations and git operations through Docker
 *
 * Prerequisites:
 * - Docker must be installed and running
 * - moatless-vibe-universal:latest image must be built (run: docker build -t moatless-vibe-universal:latest -f docker/Dockerfile .)
 * - Claude authentication (via "claude login") for full functionality
 *
 * Note:
 * - The test uses host.docker.internal to allow container to communicate with host
 * - A harmless "Claude Code process exited" error may appear after tests complete
 * - If Claude is not authenticated, the test will still validate infrastructure
 */

describe('Docker Workspace E2E Test', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let serverPort: number

  beforeAll(async () => {
    // Ensure moatless-vibe-universal image exists
    try {
      await execAsync('docker image inspect moatless-vibe-universal:latest', { timeout: 5000 })
    } catch (_error) {
      throw new Error(
        'moatless-vibe-universal:latest image not found. Please build it first with: docker build -t moatless-vibe-universal:latest -f docker/Dockerfile .',
      )
    }
  })

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-docker-e2e-test-'))

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    // Enable Docker workspace as default for this test
    process.env.DEFAULT_WORKSPACE_TYPE = 'docker'

    app = await build()

    // Start the server on a random available port
    await app.listen({ port: 0, host: '127.0.0.1' })
    const serverAddress = app.server.address() as { port: number }
    serverPort = serverAddress.port

    // Set environment variables for worker mode
    process.env.API_URL = `http://host.docker.internal:${serverPort}`
    process.env.WORKER_API_URL = `http://host.docker.internal:${serverPort}`
    process.env.WORKER_API_KEY = 'test-key'

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
  })

  afterEach(async () => {
    // Stop any running agents before cleanup
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        await new Promise((resolve) => setTimeout(resolve, 100))
      } catch (_e) {
        // Agent might not be running
      }
    }

    // Cleanup task if it exists
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
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

    // Clean up any Docker containers created by this test
    try {
      const { stdout } = await execAsync(
        `docker ps -a --filter "label=moatless.task-id=${createdTaskId}" -q`,
      )
      const containerIds = stdout.trim().split('\n').filter(Boolean)
      for (const containerId of containerIds) {
        await execAsync(`docker stop ${containerId} && docker rm ${containerId}`)
      }
    } catch (_e) {
      // Containers might already be cleaned up
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }

    // Clean up environment variables
    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    delete process.env.DEFAULT_WORKSPACE_TYPE
    delete process.env.API_URL
    delete process.env.WORKER_API_URL
    delete process.env.WORKER_API_KEY

    // Wait for cleanup to complete
    await new Promise((resolve) => setTimeout(resolve, 1000))

    await app.close()
  }, 120000) // 2 minute timeout for cleanup

  it('should complete the full Docker workflow: create repository, create task, execute in container, and verify results', async () => {
    // Step 1: Create repository with Docker workspace configuration
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker E2E Test Repository',
      description: 'Test repository for Docker workspace validation',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'docker',
        dockerImage: 'moatless-vibe-universal:latest',
        setupCommands: 'echo "Docker workspace setup complete"',
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    logger.info('Created repository with Docker workspace', repository)

    createdRepositoryId = repository.id

    expect(repository).toMatchObject({
      name: createRepositoryRequest.name,
      description: createRepositoryRequest.description,
      path: createRepositoryRequest.path,
      useGitWorktrees: createRepositoryRequest.useGitWorktrees,
      codeExecution: expect.objectContaining({
        type: 'docker',
        dockerImage: 'moatless-vibe-universal:latest',
      }),
    })

    // Step 2: Create and start a task (this will create Docker container)
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Create a file called docker_test.py with the content: print("Hello from Docker!")',
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
    logger.info('Created task with Docker workspace', taskCreationResult)

    // Wait for Docker container setup and workspace initialization
    await new Promise((resolve) => setTimeout(resolve, 5000))

    // Step 3: Verify Docker container was created
    const { stdout: containerList } = await execAsync(
      `docker ps -a --filter "label=moatless.task-id=${createdTaskId}" --format "{{.Names}}"`,
    )
    const containerName = containerList.trim()
    expect(containerName).toBeTruthy()
    expect(containerName).toContain('moatless-docker-e2e-test-repository')
    logger.info('Docker container created:', { containerName })

    // Step 4: Verify task was created and has initial state
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(getTaskResponse.statusCode).toBe(200)
    const task: Task = JSON.parse(getTaskResponse.body)
    expect(task.taskId).toBe(createdTaskId)
    expect(task.repositoryId).toBe(repository.id)
    expect(task.agentType).toBe('claude-code')
    expect(['open', 'running']).toContain(task.status)

    // Step 5: Verify workspace status
    const workspaceStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/workspace-status`,
    })

    expect(workspaceStatusResponse.statusCode).toBe(200)
    const workspaceStatus = JSON.parse(workspaceStatusResponse.body)
    // Workspace status endpoint returns status, gitStatus, and backgroundTaskCount
    expect(['idle', 'running', 'ready']).toContain(workspaceStatus.status)
    logger.info('Workspace status:', workspaceStatus)

    // Step 6: Poll events to track task progress
    let pollAttempts = 0
    const maxPollAttempts = 60 // 60 seconds max for Docker agent to complete
    let resultEvent: Event | null = null
    let hasStartedProcessing = false
    let errorEvent: Event | null = null
    const toolUseEvents: Event[] = []
    const workspaceEvents: Event[] = []

    while (pollAttempts < maxPollAttempts && !resultEvent && !errorEvent) {
      const eventsResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/events`,
      })

      expect(eventsResponse.statusCode).toBe(200)
      const { events } = JSON.parse(eventsResponse.body)

      // Look for key events
      for (const event of events) {
        if (event.type === 'workspace') {
          workspaceEvents.push(event)
          console.log('Workspace event:', event.subType, event.payload)
        }
        if (event.type === 'message') {
          hasStartedProcessing = true
          // Log tool use messages
          if (event.payload?.type === 'tool_use') {
            console.log('Tool use:', event.payload.name, event.payload.params)
            toolUseEvents.push(event)
          }
        }
        if (event.type === 'result') {
          if (!event.isError) {
            resultEvent = event
          } else {
            errorEvent = event
            console.error('Result error event:', event.payload)
          }
          break
        }
        if (event.isError) {
          console.warn(`Error event (${event.type}):`, event.payload)
        }
      }

      if (resultEvent || errorEvent) break

      // Wait 1 second before next poll
      await new Promise((resolve) => setTimeout(resolve, 1000))
      pollAttempts++
    }

    // Step 7: Verify processing completed successfully
    expect(hasStartedProcessing).toBe(true)
    expect(resultEvent).toBeTruthy()
    expect(errorEvent).toBeFalsy()

    // Verify we got workspace setup events
    const setupEvents = workspaceEvents.filter((e) => e.subType === 'setup_completed')
    expect(setupEvents.length).toBeGreaterThan(0)
    expect(setupEvents[0].payload.containerId).toBeTruthy()

    // Log the result for debugging
    if (resultEvent) {
      console.log('Agent completed successfully in Docker container:', {
        totalCost: resultEvent.payload?.total_cost_usd,
        hasResult: !!resultEvent.payload?.result,
      })
    }

    // Step 8: Verify the file was created in the mounted workspace and is accessible on host
    const dockerTestPath = path.join(tempDir, 'docker_test.py')

    // Verify file exists on host filesystem
    const fileExists = await fs
      .access(dockerTestPath)
      .then(() => true)
      .catch(() => false)
    expect(fileExists).toBe(true)
    console.log('✅ File created in Docker container is accessible on host filesystem')

    // Read file content from host
    const fileContent = await fs.readFile(dockerTestPath, 'utf-8')
    console.log('File content read from host:', fileContent)
    expect(fileContent).toContain('Hello from Docker!')
    expect(fileContent).toContain('print')

    // Verify file stats (should be accessible with normal permissions)
    const fileStats = await fs.stat(dockerTestPath)
    expect(fileStats.isFile()).toBe(true)
    expect(fileStats.size).toBeGreaterThan(0)
    console.log('File stats:', {
      size: fileStats.size,
      mode: fileStats.mode.toString(8),
      uid: fileStats.uid,
      gid: fileStats.gid,
    })

    // Verify we can write to the file from host (proving full access)
    const appendContent = '\n# Added from host'
    await fs.appendFile(dockerTestPath, appendContent)
    const updatedContent = await fs.readFile(dockerTestPath, 'utf-8')
    expect(updatedContent).toContain(appendContent)
    console.log('✅ Successfully modified Docker-created file from host')

    // Verify the file is visible to git on the host
    const { stdout: gitStatus } = await execAsync('git status --porcelain', { cwd: tempDir })
    expect(gitStatus).toContain('docker_test.py')
    console.log('✅ Git on host sees the Docker-created file:', gitStatus.trim())

    // Step 8.5: Verify file can be accessed through the workspace API
    const readFileResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/files/docker_test.py`,
    })
    expect(readFileResponse.statusCode).toBe(200)
    const readFileResult = JSON.parse(readFileResponse.body)
    expect(readFileResult.content).toContain('Hello from Docker!')
    expect(readFileResult.content).toContain('Added from host')
    console.log('✅ File created in Docker is readable through workspace API')

    // List files to verify it appears in directory listing
    const listFilesResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/files?recursive=true`,
    })
    expect(listFilesResponse.statusCode).toBe(200)
    const { files } = JSON.parse(listFilesResponse.body)
    // Files are returned with leading slashes
    expect(files).toContain('/docker_test.py')
    console.log('✅ Docker-created file appears in workspace file listing:', files)

    // Step 9: Verify git diff
    const gitDiffResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/git-diff`,
    })

    expect(gitDiffResponse.statusCode).toBe(200)
    const gitDiff = JSON.parse(gitDiffResponse.body)
    expect(gitDiff).toBeDefined()

    if (gitDiff.diff) {
      expect(gitDiff.diff).toContain('docker_test.py')
      expect(gitDiff.diff).toContain('Hello from Docker!')
      console.log('Git diff shows changes made in Docker container')
    }

    // Step 10: Get final task state and verify iteration results
    const finalTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(finalTaskResponse.statusCode).toBe(200)
    const finalTask: Task = JSON.parse(finalTaskResponse.body)

    expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)
    expect(finalTask.numTurns).toBeGreaterThanOrEqual(1)

    const firstIteration = finalTask.iterations[0]
    expect(firstIteration.status).toBe('completed')
    expect(firstIteration.agentSessionId).toBeDefined()
    expect(firstIteration.result).toBeTruthy()
    expect(firstIteration.costUsd).toBeGreaterThan(0)
    expect(firstIteration.durationMs).toBeGreaterThan(0)

    console.log('Docker task iteration completed:', {
      status: firstIteration.status,
      costUsd: firstIteration.costUsd,
      durationMs: firstIteration.durationMs,
      resultLength: firstIteration.result?.length,
    })

    // Step 11: Verify container is still running after task completion
    const { stdout: containerStatus } = await execAsync(
      `docker ps --filter "name=${containerName}" --format "{{.Status}}"`,
    )
    expect(containerStatus).toContain('Up')
    console.log('Docker container still running:', containerStatus.trim())

    // Step 12: Verify events were properly recorded
    const allEventsResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/events?limit=200`,
    })

    expect(allEventsResponse.statusCode).toBe(200)
    const { events: allEvents } = JSON.parse(allEventsResponse.body)

    // Should have workspace events from Docker container
    const dockerWorkspaceEvents = allEvents.filter(
      (e: Event) => e.type === 'workspace' && e.agentType === 'system',
    )
    expect(dockerWorkspaceEvents.length).toBeGreaterThan(0)

    // Should have message events from agent running in container
    const messageEvents = allEvents.filter((e: Event) => e.type === 'message')
    expect(messageEvents.length).toBeGreaterThan(0)

    console.log('✅ Docker workspace E2E test completed successfully:', {
      totalEvents: allEvents.length,
      workspaceEvents: dockerWorkspaceEvents.length,
      messageEvents: messageEvents.length,
      containerCreated: true,
      agentExecutedInContainer: true,
      fileCreatedThroughDocker: true,
      eventsRecorded: true,
    })
  }, 90000) // 90 second timeout for Docker operations

  it('should handle multiple tasks with Docker workspaces concurrently', async () => {
    // Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker E2E Test Repository - Concurrent',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'docker',
        dockerImage: 'moatless-vibe-universal:latest',
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Create two tasks simultaneously
    const taskIds: string[] = []

    const createTaskPromises = [1, 2].map(async (num) => {
      const createTaskRequest: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: repository.id,
        message: `Create a file called concurrent_test_${num}.txt with "Task ${num} from Docker"`,
        start: true,
      }

      const response = await app.inject({
        method: 'POST',
        url: '/api/v1/tasks',
        payload: createTaskRequest,
      })

      const result = JSON.parse(response.body)
      taskIds.push(result.taskId)
      return result
    })

    await Promise.all(createTaskPromises)

    // Wait for containers to be created
    await new Promise((resolve) => setTimeout(resolve, 5000))

    // Verify both containers were created
    for (const taskId of taskIds) {
      const { stdout } = await execAsync(
        `docker ps -a --filter "label=moatless.task-id=${taskId}" --format "{{.Names}}"`,
      )
      expect(stdout.trim()).toBeTruthy()
      console.log(`Container created for task ${taskId}: ${stdout.trim()}`)
    }

    // Cleanup
    for (const taskId of taskIds) {
      try {
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${taskId}/stop`,
        })
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${taskId}`,
        })
      } catch (_e) {
        // Ignore cleanup errors
      }
    }
  }, 60000)

  it('should properly clean up Docker containers on task deletion', async () => {
    // Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker E2E Test Repository - Cleanup',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'docker',
        dockerImage: 'moatless-vibe-universal:latest',
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Create task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Create cleanup_test.txt',
      start: true,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    const taskResult = JSON.parse(createTaskResponse.body)
    const taskId = taskResult.taskId

    // Wait for container to be created
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Verify container exists
    const { stdout: beforeDelete } = await execAsync(
      `docker ps -a --filter "label=moatless.task-id=${taskId}" -q`,
    )
    expect(beforeDelete.trim()).toBeTruthy()

    // Delete task (should clean up container)
    const deleteResponse = await app.inject({
      method: 'DELETE',
      url: `/api/v1/tasks/${taskId}`,
    })
    expect(deleteResponse.statusCode).toBe(200)

    // Wait for cleanup
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Verify container was removed
    const { stdout: afterDelete } = await execAsync(
      `docker ps -a --filter "label=moatless.task-id=${taskId}" -q`,
    )
    expect(afterDelete.trim()).toBe('')
    console.log('✅ Docker container properly cleaned up after task deletion')
  }, 30000)
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
    '# Docker Test Repository\n\nThis is a test repository for Docker workspace E2E testing.\n',
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
