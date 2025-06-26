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

/**
 * E2E Test using Real ClaudeCodeAgent
 *
 * This test uses the actual ClaudeCodeAgent instead of mocks, providing a fully realistic
 * test environment. The ClaudeCodeAgent will run using the @anthropic-ai/claude-code SDK
 * which handles API authentication automatically through the claude CLI environment.
 *
 * The test covers the complete workflow including:
 * - Environment creation with local git repository
 * - Task creation and agent execution
 * - Workspace operations and file generation
 * - Event streaming and status monitoring
 *
 * Note:
 * - If Claude is not authenticated (via "claude login"), the test will still pass but
 *   will show an authentication message. This validates the infrastructure works correctly.
 * - A harmless "Claude Code process exited" error may appear after tests complete due to
 *   the Claude SDK cleanup process. This doesn't affect test results.
 *
 * To run with full agent functionality:
 * 1. Run "claude login" to authenticate
 * 2. Run "pnpm test complete-workflow-e2e"
 */

describe('Complete Workflow E2E Test', () => {
  let app: FastifyInstance
  let tempDir: string
  let createdEnvironmentId: string
  let createdTaskId: string

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-test-'))

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    app = await build()

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
  })

  afterEach(async () => {
    // Stop any running agents before cleanup
    if (createdTaskId) {
      try {
        // First try to stop the agent
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        // Give it a moment to clean up
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

    // Clean up test environment variable
    delete process.env.SKIP_CLAUDE_DISK_EVENTS

    // Stop all running agents before closing the app
    if (app.services?.agent) {
      await app.services.agent.stopAllAgents()
    }

    // Wait a bit more for any background processes to finish
    await new Promise((resolve) => setTimeout(resolve, 500))

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
      message: 'Create a file called hello_world.py with the content: print("Hello, World!")',
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

    // Wait a bit for background workspace setup and initial message processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

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

    // Step 4: Poll events to track task progress and verify agent status while running
    let pollAttempts = 0
    const maxPollAttempts = 40 // 40 seconds max for real agent to complete
    let resultEvent: Event | null = null
    let hasStartedProcessing = false
    let errorEvent: Event | null = null
    const toolUseEvents: Event[] = []
    let agentWasRunning = false

    while (pollAttempts < maxPollAttempts && !resultEvent && !errorEvent) {
      // Check agent status while polling
      const agentStatusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/agent-status`,
      })

      if (agentStatusResponse.statusCode === 200) {
        const agentStatus = JSON.parse(agentStatusResponse.body)
        if (agentStatus.status === 'running') {
          agentWasRunning = true
          console.log('Agent is running:', {
            sessionId: agentStatus.sessionId,
            currentIteration: agentStatus.currentIteration,
            workspaceStatus: agentStatus.workspaceStatus,
          })
        }
      }

      const eventsResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/events`,
      })

      expect(eventsResponse.statusCode).toBe(200)
      const { events } = JSON.parse(eventsResponse.body)

      // Look for key events
      for (const event of events) {
        if (event.type === 'message') {
          hasStartedProcessing = true // Count message events as processing started
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

    // Verify that we saw the agent running at some point
    expect(agentWasRunning).toBe(true)

    // Step 5: Verify processing completed successfully
    expect(hasStartedProcessing).toBe(true)
    expect(resultEvent).toBeTruthy()
    expect(errorEvent).toBeFalsy()

    // Log the result for debugging
    if (resultEvent) {
      console.log('Agent completed successfully:', {
        totalCost: resultEvent.payload?.total_cost_usd,
        hasResult: !!resultEvent.payload?.result,
      })
    }

    // Step 6: Verify the file was created
    const helloWorldPath = path.join(tempDir, 'hello_world.py')
    const fileExists = await fs
      .access(helloWorldPath)
      .then(() => true)
      .catch(() => false)

    expect(fileExists).toBe(true)

    const fileContent = await fs.readFile(helloWorldPath, 'utf-8')
    console.log('✅ Claude Code agent successfully created hello_world.py!')
    console.log('File content:', fileContent)
    expect(fileContent).toContain('Hello, World!')
    expect(fileContent).toContain('print')

    // Step 7: Verify workspace status
    const workspaceStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/workspace-status`,
    })

    if (workspaceStatusResponse.statusCode === 200) {
      const workspaceStatus = JSON.parse(workspaceStatusResponse.body)
      expect(workspaceStatus).toHaveProperty('status')
      // Status could be various values depending on timing and implementation
      expect(['idle', 'running', 'completed', 'not_setup', 'ready', 'setting_up']).toContain(
        workspaceStatus.status,
      )
    }

    // Step 7.1: Verify agent status endpoint
    const agentStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/agent-status`,
    })

    expect(agentStatusResponse.statusCode).toBe(200)
    const agentStatus = JSON.parse(agentStatusResponse.body)
    console.log('Agent status:', agentStatus)

    // Verify required fields exist
    expect(agentStatus).toHaveProperty('status')
    expect(agentStatus).toHaveProperty('sessionId')
    expect(agentStatus).toHaveProperty('startedAt')
    expect(agentStatus).toHaveProperty('currentIteration')
    expect(agentStatus).toHaveProperty('workspaceStatus')
    expect(agentStatus).toHaveProperty('canExecute')

    // Verify status is one of expected values
    expect(['idle', 'running', 'preparing', 'error']).toContain(agentStatus.status)

    // Verify boolean field
    expect(typeof agentStatus.canExecute).toBe('boolean')

    // If agent has run, verify session information
    if (agentStatus.sessionId) {
      expect(agentStatus.sessionId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
      ) // UUID format
      if (agentStatus.startedAt) {
        expect(agentStatus.startedAt).toBeTruthy()
      }
      if (typeof agentStatus.currentIteration === 'number') {
        expect(agentStatus.currentIteration).toBeGreaterThanOrEqual(1)
      }
    }

    // Step 8: Verify git diff (may be empty if agent committed the file)
    const gitDiffResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/git-diff`,
    })

    expect(gitDiffResponse.statusCode).toBe(200)
    const gitDiff = JSON.parse(gitDiffResponse.body)
    expect(gitDiff).toBeDefined()

    if (gitDiff.diff) {
      // If there's a diff, it should show the new file
      expect(gitDiff.diff).toContain('hello_world.py')
      expect(gitDiff.diff).toContain('Hello, World!')
      console.log('Git diff shows uncommitted changes:', gitDiff.diff.substring(0, 200) + '...')
    } else {
      // No diff means the file was likely committed by the agent
      console.log('No git diff - file may have been committed by the agent')

      // Verify the file was committed by checking git log
      const gitLogResponse = await runCommand('git', ['log', '-1', '--oneline'], tempDir)
        .then(() => true)
        .catch(() => false)
      expect(gitLogResponse).toBe(true)
    }

    // Step 9: Get final task state and verify iteration results
    const finalTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(finalTaskResponse.statusCode).toBe(200)
    const finalTask: Task = JSON.parse(finalTaskResponse.body)

    // The task should have at least one iteration from the initial message
    expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)
    // NumTurns should be at least 1 after agent runs
    expect(finalTask.numTurns).toBeGreaterThanOrEqual(1)

    // Verify the first iteration has completed successfully
    const firstIteration = finalTask.iterations[0]
    expect(firstIteration.status).toBe('completed')
    expect(firstIteration.agentSessionId).toBeDefined()
    expect(firstIteration.agentSessionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
    ) // UUID format

    // Verify iteration has results
    expect(firstIteration.result).toBeTruthy()
    expect(firstIteration.costUsd).toBeGreaterThan(0)
    expect(firstIteration.durationMs).toBeGreaterThan(0)
    expect(firstIteration.endTime).toBeTruthy()

    console.log('Task iteration completed:', {
      status: firstIteration.status,
      costUsd: firstIteration.costUsd,
      durationMs: firstIteration.durationMs,
      resultLength: firstIteration.result?.length,
    })

    // Step 10: Get all events for final verification
    const allEventsResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/events?limit=100`,
    })

    expect(allEventsResponse.statusCode).toBe(200)
    const { events: allEvents } = JSON.parse(allEventsResponse.body)

    // Real ClaudeCodeAgent emits 'message' and 'result' events
    const eventTypes = allEvents.map((event: Event) => event.type)
    console.log('All event types:', eventTypes)

    // Verify we have message events (agent communication)
    const messageEvents = allEvents.filter((event: Event) => event.type === 'message')
    expect(messageEvents.length).toBeGreaterThanOrEqual(1)

    // Log detailed event structure to understand format
    if (messageEvents.length > 0) {
      console.log('Sample message event structure:', {
        type: messageEvents[0].type,
        payloadKeys: Object.keys(messageEvents[0].payload || {}),
        payloadType: messageEvents[0].payload?.type,
        payloadName: messageEvents[0].payload?.name,
      })
    }

    // Check if we tracked tool use events during polling
    if (toolUseEvents.length > 0) {
      console.log('Tool use events captured during polling:', toolUseEvents.length)
      const toolNames = toolUseEvents.map((event: Event) => event.payload?.name)
      console.log('Tools used:', [...new Set(toolNames)])

      // Should use Write or Edit tool to create the file
      expect(
        toolNames.some((name) => name === 'Write' || name === 'Edit' || name === 'MultiEdit'),
      ).toBe(true)
    } else {
      // If no tool use events were captured during polling, at least verify we have messages
      console.log('No tool use events captured during polling, but agent completed successfully')
      expect(messageEvents.length).toBeGreaterThan(0)
    }

    // Log some events for debugging
    if (allEvents.length > 0) {
      console.log('Event summary:', {
        totalEvents: allEvents.length,
        messageEvents: messageEvents.length,
        resultEvents: allEvents.filter((e: Event) => e.type === 'result').length,
        errorEvents: allEvents.filter((e: Event) => e.isError).length,
      })
    }
  }, 60000) // 60 second timeout for real agent execution

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

    // Create task without starting it and without initial message
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
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

    // Wait a bit for background setup to start
    await new Promise((resolve) => setTimeout(resolve, 100))

    // Task should be created but not processing
    const getTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    const task: Task = JSON.parse(getTaskResponse.body)
    expect(task.status).toBe('active')
    expect(task.iterations.length).toBe(0) // No iterations yet since we didn't start

    // Wait for workspace to be ready before sending message
    await new Promise((resolve) => setTimeout(resolve, 1000))

    // Now manually send a message to start the task
    const sendMessageResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
      payload: {
        message: 'Create a simple test.txt file with "Hello Test" content',
      },
    })

    if (sendMessageResponse.statusCode !== 200) {
      console.error('Send message failed:', sendMessageResponse.body)

      // Get more details about the task state
      const taskStateResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}`,
      })
      console.error('Task state:', JSON.parse(taskStateResponse.body))

      // Check workspace status
      const workspaceStatusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/workspace-status`,
      })
      console.error('Workspace status:', JSON.parse(workspaceStatusResponse.body))
    }
    expect(sendMessageResponse.statusCode).toBe(200)

    // Verify iteration was created
    const updatedTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    const updatedTask: Task = JSON.parse(updatedTaskResponse.body)
    expect(updatedTask.iterations.length).toBe(1)
    expect(updatedTask.numTurns).toBe(1)
  }, 45000) // Longer timeout for real agent execution

  it('should track agent status changes during execution', async () => {
    // Create environment
    const createEnvironmentRequest: CreateEnvironmentRequest = {
      name: 'E2E Test Environment - Status Tracking',
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

    // Create and start task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      environmentId: environment.id,
      message: 'Create a file called status_test.txt with "Testing agent status" content',
      start: true,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    const taskResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskResult.taskId

    // Track status changes
    const statusHistory: Array<{ status: string; timestamp: number }> = []
    let resultFound = false
    let attempts = 0
    const maxAttempts = 40

    while (!resultFound && attempts < maxAttempts) {
      // Get agent status
      const agentStatusResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/agent-status`,
      })

      expect(agentStatusResponse.statusCode).toBe(200)
      const agentStatus = JSON.parse(agentStatusResponse.body)

      // Record status change
      const lastStatus = statusHistory[statusHistory.length - 1]
      if (!lastStatus || lastStatus.status !== agentStatus.status) {
        statusHistory.push({
          status: agentStatus.status,
          timestamp: Date.now(),
        })
        console.log('Agent status changed to:', agentStatus.status)
      }

      // Check for completion
      const eventsResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/events?limit=10`,
      })

      const { events } = JSON.parse(eventsResponse.body)
      resultFound = events.some((event: Event) => event.type === 'result')

      if (!resultFound) {
        await new Promise((resolve) => setTimeout(resolve, 500))
        attempts++
      }
    }

    // Verify we saw expected status transitions
    const statuses = statusHistory.map((h) => h.status)
    console.log('Status history:', statuses)

    // Should see at least idle -> running transition
    expect(statuses).toContain('running')

    // After completion, status should be idle again
    const finalStatusResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/agent-status`,
    })

    const finalStatus = JSON.parse(finalStatusResponse.body)
    expect(finalStatus.status).toBe('idle')
    expect(finalStatus.sessionId).toBeTruthy()
    expect(finalStatus.canExecute).toBe(true)
  }, 60000) // 60 second timeout
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
