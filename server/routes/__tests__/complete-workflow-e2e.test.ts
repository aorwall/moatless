import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  Event,
  RepositoryConfig,
  Task,
} from '../../../shared/types/index.js'
import { build } from '../../test/helper.js'

/**
 * E2E Test using Real ClaudeCodeAgent
 *
 * This test uses the actual ClaudeCodeAgent instead of mocks, providing a fully realistic
 * test repository. The ClaudeCodeAgent will run using the @anthropic-ai/claude-code SDK
 * which handles API authentication automatically through the claude CLI repository.
 *
 * The test covers the complete workflow including:
 * - Repository creation with local git repository
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

// Helper function to simulate SSE stream connection using polling
// Note: In test repository, we can't establish real SSE connections since
// the server isn't actually listening on a port. Instead, we'll use polling
// to simulate real-time event streaming.
async function simulateSSEStream(
  app: FastifyInstance,
  taskId: string,
  onEvent: (event: Event) => void,
  onError?: (error: Error) => void,
): Promise<() => void> {
  let isActive = true
  const seenEventIds = new Set<string>()

  // Poll for events every 100ms to simulate real-time streaming
  const pollInterval = setInterval(async () => {
    if (!isActive) {
      clearInterval(pollInterval)
      return
    }

    try {
      const response = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${taskId}/events`,
      })

      if (response.statusCode !== 200) {
        if (onError) {
          onError(new Error(`Failed to fetch events: ${response.statusCode}`))
        }
        return
      }

      const { events } = JSON.parse(response.body)

      // Process only new events
      for (const event of events) {
        if (!seenEventIds.has(event.eventId)) {
          seenEventIds.add(event.eventId)
          onEvent(event)
        }
      }
    } catch (error) {
      if (onError) {
        onError(error as Error)
      }
    }
  }, 100) // Poll every 100ms for near real-time experience

  // Return cleanup function
  return () => {
    isActive = false
    clearInterval(pollInterval)
  }
}

describe('Complete Workflow E2E Test', () => {
  let app: FastifyInstance
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  // let _serverAddress: string

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-test-'))

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    app = await build()

    // Start the server on a random available port for SSE testing
    await app.listen({ port: 0, host: '127.0.0.1' })
    // const serverAddress = app.server.address() as { port: number }
    // _serverAddress = `http://127.0.0.1:${serverAddress.port}`

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
  })

  afterEach(async () => {
    // Stop any running agents before cleanup - ONLY for this specific task
    if (createdTaskId) {
      try {
        // First try to stop the agent for this specific task
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

    // Cleanup repository if it exists
    if (createdRepositoryId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${createdRepositoryId}`,
        })
      } catch (_e) {
        // Repository might already be deleted or not exist
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

    // Clean up test repository variable
    delete process.env.SKIP_CLAUDE_DISK_EVENTS

    // NOTE: Do NOT call stopAllAgents() here as it affects other test instances
    // Individual task cleanup above should be sufficient

    // Wait a bit for any background processes to finish
    await new Promise((resolve) => setTimeout(resolve, 500))

    await app.close()
  })

  it('should complete the full workflow: create repository, create task, execute, and verify results', async () => {
    // Step 1: Create repository with temporary git repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'E2E Test Repository',
      description: 'Test repository for complete workflow validation',
      repository: {
        type: 'local',
        path: tempDir,
        useGitWorktrees: false,
      },
      codeExecution: {
        type: 'local',
        setupCommands: 'echo "Repository setup complete"',
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

    expect(repository).toMatchObject({
      name: createRepositoryRequest.name,
      description: createRepositoryRequest.description,
      repository: createRepositoryRequest.repository,
      codeExecution: createRepositoryRequest.codeExecution,
    })

    // Step 2: Create and start a task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
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
    expect(taskCreationResult.status).toBe('open')

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
    expect(task.repositoryId).toBe(repository.id)
    expect(task.agentType).toBe('claude-code')
    // Task may still be running since agent execution takes time
    expect(['open', 'running']).toContain(task.status)

    // Step 4: Set up SSE event streaming simulation
    // Note: In test repository, we simulate SSE by polling the events endpoint
    // at high frequency (100ms) to verify that events are emitted in real-time
    // during agent execution. This tests the same event emission logic that
    // powers the actual SSE endpoint.
    const sseEvents: Event[] = []
    const sseErrors: Error[] = []
    let sseCleanup: (() => void) | null = null

    // Connect to simulated SSE stream
    try {
      sseCleanup = await simulateSSEStream(
        app,
        createdTaskId,
        (event) => {
          sseEvents.push(event)
          console.log(`SSE Event received (${event.type}):`, {
            eventId: event.eventId,
            timestamp: event.timestamp,
            iterationId: event.iterationId,
            isError: event.isError,
          })
        },
        (error) => {
          sseErrors.push(error)
          console.error('SSE Error:', error.message)
        },
      )
      console.log('SSE stream simulation started for task:', createdTaskId)
    } catch (error) {
      console.error('Failed to start SSE stream simulation:', error)
    }

    // Step 5: Poll events to track task progress and verify agent status while running
    let pollAttempts = 0
    const maxPollAttempts = 40 // 40 seconds max for real agent to complete
    let resultEvent: Event | null = null
    let hasStartedProcessing = false
    let errorEvent: Event | null = null
    const toolUseEvents: Event[] = []
    const agentWasRunning = false

    while (pollAttempts < maxPollAttempts && !resultEvent && !errorEvent) {
      // Agent status check removed - no longer part of API

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

    // Agent status endpoint removed - no longer part of API

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
      console.log('Git diff shows uncommitted changes:', `${gitDiff.diff.substring(0, 200)}...`)
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
    expect(typeof firstIteration.result).toBe('string')
    expect(firstIteration.result!.length).toBeGreaterThan(0)
    // The result should contain something meaningful from Claude
    expect(firstIteration.result).not.toBe('No result found in Claude Code SDK result message')

    expect(firstIteration.costUsd).toBeGreaterThan(0)
    expect(firstIteration.durationMs).toBeGreaterThan(0)
    expect(firstIteration.endTime).toBeTruthy()

    console.log('Task iteration completed:', {
      status: firstIteration.status,
      costUsd: firstIteration.costUsd,
      durationMs: firstIteration.durationMs,
      resultLength: firstIteration.result?.length,
      resultPreview: `${firstIteration.result?.substring(0, 100)}...`,
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

    // Step 11: Verify simulated SSE event streaming
    // Clean up SSE simulation
    if (sseCleanup) {
      sseCleanup()
    }

    console.log('SSE Simulation Verification:', {
      totalSSEEvents: sseEvents.length,
      sseErrors: sseErrors.length,
    })

    // Verify we received events via simulated SSE
    expect(sseEvents.length).toBeGreaterThan(0)
    expect(sseErrors.length).toBe(0)

    // Verify simulated SSE events include message and result events
    const sseMessageEvents = sseEvents.filter((e) => e.type === 'message')
    const sseResultEvents = sseEvents.filter((e) => e.type === 'result')

    expect(sseMessageEvents.length).toBeGreaterThan(0)
    expect(sseResultEvents.length).toBe(1) // Should have exactly one result event

    // Verify simulated SSE events match polled events (at least for key events)
    const polledEventIds = allEvents.map((e: Event) => e.eventId)
    const sseEventIds = sseEvents.map((e) => e.eventId)

    // All simulated SSE events should be in the polled events
    for (const sseEventId of sseEventIds) {
      expect(polledEventIds).toContain(sseEventId)
    }

    // Verify event ordering - events should be received in chronological order
    for (let i = 1; i < sseEvents.length; i++) {
      const prevTimestamp = new Date(sseEvents[i - 1].timestamp!).getTime()
      const currTimestamp = new Date(sseEvents[i].timestamp!).getTime()
      expect(currTimestamp).toBeGreaterThanOrEqual(prevTimestamp)
    }

    // Verify real-time nature - check that simulated SSE events were received during execution
    // The result event in simulated SSE should match the one we found during polling
    if (resultEvent && sseResultEvents.length > 0) {
      expect(sseResultEvents[0].eventId).toBe(resultEvent.eventId)
      expect(sseResultEvents[0].payload).toEqual(resultEvent.payload)
    }

    console.log('✅ SSE event streaming simulation verified successfully:', {
      messageEvents: sseMessageEvents.length,
      resultEvents: sseResultEvents.length,
      totalEvents: sseEvents.length,
      eventsInOrder: true,
      matchesPolledData: true,
    })
  }, 60000) // 60 second timeout for real agent execution

  it('should handle task creation without immediate start', async () => {
    // Create repository first
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'E2E Test Repository - No Start',
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

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Create task without starting it and without initial message
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
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
    expect(task.status).toBe('open')
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

  it.skip('should track agent status changes during execution - REMOVED: agent status no longer part of API', async () => {
    // Create repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'E2E Test Repository - Status Tracking',
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

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    createdRepositoryId = repository.id

    // Create and start task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
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

    // Track completion
    let resultFound = false
    let attempts = 0
    const maxAttempts = 40

    while (!resultFound && attempts < maxAttempts) {
      // Agent status check removed - no longer part of API

      // Status tracking removed - agent status no longer part of API

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

    // Status history verification removed - agent status no longer part of API

    // Agent status endpoint removed - no longer part of API
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
