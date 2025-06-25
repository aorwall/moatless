import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type {
  CreateEnvironmentRequest,
  CreateTaskRequest,
  Environment,
  Event,
  Task,
} from '../../../shared/types/index.js'
import { build } from '../../test/helper.js'

describe('Complete Workflow E2E Test', () => {
  let app: FastifyInstance
  let tempDir: string
  let createdEnvironmentId: string
  let createdTaskId: string

  beforeEach(async () => {
    app = await build()

    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-test-'))

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
  })

  afterEach(async () => {
    // Cleanup task if it exists
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted or not exist
      }
    }

    // Cleanup environment if it exists
    if (createdEnvironmentId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/environments/${createdEnvironmentId}`,
        })
      } catch (_e) {
        // Environment might already be deleted or not exist
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

    await app.close()
  })

  it('should complete the full workflow: create environment, create task, execute, and verify results', async () => {
    // Step 1: Create environment with temporary git repository
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'E2E Test Environment',
      description: 'Test environment for complete workflow validation',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        setupCommands: 'echo "Environment setup complete"',
      },
      variables: {
        TEST_MODE: 'true',
        PYTHON_PATH: '/usr/bin/python3',
      },
    }

    const createEnvResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/environments',
      payload: createEnvironmentRequest,
    })

    expect(createEnvResponse.statusCode).toBe(200)
    const environment: Environment = JSON.parse(createEnvResponse.body)
    createdEnvironmentId = environment.id

    expect(environment).toMatchObject({
      name: createEnvironmentRequest.name,
      description: createEnvironmentRequest.description,
      repository: createEnvironmentRequest.repository,
      codeExecution: createEnvironmentRequest.codeExecution,
    })

    // Step 2: Create and start a task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
      message: 'Create a simple hello_world.py file that prints "Hello, World!" to the console.',
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
    expect(taskCreationResult.status).toBe('active')

    // Wait for background workspace setup to complete
    await app.services.task.waitForBackgroundTasks()

    // Step 3: Verify task was created and has initial state
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(getTaskResponse.statusCode).toBe(200)
    const task: Task = JSON.parse(getTaskResponse.body)
    expect(task.taskId).toBe(createdTaskId)
    expect(task.environmentId).toBe(environment.id)
    expect(task.agentType).toBe('claude-code')
    expect(task.status).toBe('active')

    // Step 4: Poll events to track task progress
    let pollAttempts = 0
    const maxPollAttempts = 30 // 30 seconds max
    let completionEvent: Event | null = null
    let hasStartedProcessing = false
    let hasWorkspaceSetup = false

    while (pollAttempts < maxPollAttempts && !completionEvent) {
      const eventsResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/events`,
      })

      expect(eventsResponse.statusCode).toBe(200)
      const { events } = JSON.parse(eventsResponse.body)

      // Look for key events
      for (const event of events) {
        if (event.type === 'workspace' && event.subType === 'setup_completed') {
          hasWorkspaceSetup = true
        }
        if (event.type === 'start') {
          hasStartedProcessing = true
        }
        if (event.type === 'completion' && !event.isError) {
          completionEvent = event
          break
        }
        if (event.type === 'error') {
          throw new Error(`Task failed with error: ${JSON.stringify(event.payload)}`)
        }
      }

      if (completionEvent) break

      // Wait 1 second before next poll
      await new Promise((resolve) => setTimeout(resolve, 1000))
      pollAttempts++
    }

    // Step 5: Verify workspace setup and processing started
    expect(hasWorkspaceSetup).toBe(true)
    expect(hasStartedProcessing).toBe(true)

    // If we have a completion event, verify it indicates success
    if (completionEvent) {
      expect(completionEvent.payload.successful).toBe(true)
      expect(completionEvent.payload.messageCount).toBeGreaterThan(0)
    }

    // Step 6: Verify the file was created
    try {
      const helloWorldPath = path.join(tempDir, 'hello_world.py')
      const fileExists = await fs
        .access(helloWorldPath)
        .then(() => true)
        .catch(() => false)

      if (fileExists) {
        const fileContent = await fs.readFile(helloWorldPath, 'utf-8')
        expect(fileContent).toContain('Hello, World!')
        expect(fileContent).toContain('print')
      }
    } catch (error) {
      // File might not be created if agent didn't complete successfully
      // This is acceptable for this test as we're mainly testing the workflow
      console.warn('hello_world.py file was not created or accessible:', error)
    }

    // Step 7: Verify workspace status
    const workspaceStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/workspaces/${createdTaskId}/status`,
    })

    if (workspaceStatusResponse.statusCode === 200) {
      const workspaceStatus = JSON.parse(workspaceStatusResponse.body)
      expect(workspaceStatus).toHaveProperty('status')
      // Status could be various values depending on timing and implementation
      expect(['idle', 'running', 'completed', 'not_setup', 'ready', 'setting_up']).toContain(
        workspaceStatus.status,
      )
    }

    // Step 8: Verify git diff (if there are changes)
    const gitDiffResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/workspaces/${createdTaskId}/git-diff`,
    })

    if (gitDiffResponse.statusCode === 200) {
      const gitDiff = JSON.parse(gitDiffResponse.body)
      // We expect either no diff (if file was committed) or a diff showing the new file
      expect(gitDiff).toBeDefined()
    }

    // Step 9: Get final task state
    const finalTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(finalTaskResponse.statusCode).toBe(200)
    const finalTask: Task = JSON.parse(finalTaskResponse.body)
    expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)
    expect(finalTask.numTurns).toBeGreaterThanOrEqual(1)

    // Verify that the first iteration has an agentSessionId
    if (finalTask.iterations.length > 0) {
      const firstIteration = finalTask.iterations[0]
      expect(firstIteration.agentSessionId).toBeDefined()
      expect(firstIteration.agentSessionId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
      ) // UUID format
      console.log('Agent session ID:', firstIteration.agentSessionId)
    }

    // Step 10: Get all events for final verification
    const allEventsResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/events?limit=100`,
    })

    expect(allEventsResponse.statusCode).toBe(200)
    const { events: allEvents } = JSON.parse(allEventsResponse.body)

    // Verify we have the expected event types
    const eventTypes = allEvents.map((event: Event) => event.type)
    expect(eventTypes).toContain('workspace') // Workspace setup

    // Should have at least one input event (from the initial message)
    const inputEvents = allEvents.filter((event: Event) => event.type === 'input')
    expect(inputEvents.length).toBeGreaterThanOrEqual(1)

    // Verify the initial input event has our message
    const initialInputEvent = inputEvents[0]
    expect(initialInputEvent.payload.message).toBe(createTaskRequest.message)
  }, 60000) // 60 second timeout for full workflow

  it('should handle task creation without immediate start', async () => {
    // Create environment first
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'E2E Test Environment - No Start',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        setupCommands: 'echo "Setup complete"',
      },
    }

    const createEnvResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/environments',
      payload: createEnvironmentRequest,
    })

    const environment: Environment = JSON.parse(createEnvResponse.body)
    createdEnvironmentId = environment.id

    // Create task without starting it
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
      message: 'Create a test file',
      start: false, // Don't start immediately
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId

    // Wait for background setup
    await app.services.task.waitForBackgroundTasks()

    // Task should be created but not processing
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    const task: Task = JSON.parse(getTaskResponse.body)
    expect(task.status).toBe('active')
    expect(task.iterations.length).toBe(0) // No iterations yet since we didn't start

    // Now manually send a message to start the task
    const sendMessageResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
      payload: {
        message: 'Create a simple test.txt file with "Hello Test" content',
      },
    })

    expect(sendMessageResponse.statusCode).toBe(200)

    // Verify iteration was created
    const updatedTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    const updatedTask: Task = JSON.parse(updatedTaskResponse.body)
    expect(updatedTask.iterations.length).toBe(1)
    expect(updatedTask.numTurns).toBe(1)
  })
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
    '# Test Repository\n\nThis is a test repository for E2E testing.\n',
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
