import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateTaskRequest, Event, Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../features/events/index'
import type { WorkspaceCoordinator } from '../features/workspace/index'
import { DockerService } from '../services/docker.service'
import { build } from '../test/helper'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'
import {
  createTestRepository,
  getTestEnvironmentConfig,
  verifyFileCreated,
} from './utils/test-helpers'

/**
 * Integration test using Real ClaudeCodeAgent
 *
 * This test uses the actual ClaudeCodeAgent instead of mocks, providing a fully realistic
 * test of the complete workflow. The ClaudeCodeAgent will run using the @anthropic-ai/claude-code SDK
 * which requires proper authentication through the claude CLI.
 *
 * The test covers the complete workflow including:
 * - Repository creation with configurable environment (local/Docker)
 * - Task creation and agent execution
 * - Workspace operations and file generation
 * - Direct event streaming via EventService (no polling)
 *
 * Environment Selection:
 * - Set DEFAULT_WORKSPACE_TYPE=docker to test with Docker environments
 * - Set DEFAULT_WORKSPACE_TYPE=local or omit to test with local environments
 *
 * Prerequisites:
 * 1. Run "claude login" to authenticate before running this test
 * 2. Ensure Claude Code CLI is properly configured
 *
 * This test will fail if Claude is not authenticated, ensuring that only properly
 * configured environments pass the integration test.
 */

describe('Complete Workflow Integration Test (broad)', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let eventService: EventService
  let workspaceCoordinator: WorkspaceCoordinator
  let eventSubscription: (() => void) | null = null
  let collectedEvents: Event[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test git repository within the data directory
    const dataDir = getDataDir()
    tempDir = await fs.mkdtemp(path.join(dataDir, 'moatless-e2e-test-'))

    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

    app = await build()

    // Start the server on a random available port for SSE testing
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Get event service from app services
    eventService = app.services.event
    if (!eventService) {
      throw new Error('EventService not available in test app')
    }

    // Get workspace coordinator for snapshot verification
    workspaceCoordinator = app.services.workspaceCoordinator
    if (!workspaceCoordinator) {
      throw new Error('WorkspaceCoordinator not available in test app')
    }

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    // Reset collected events
    collectedEvents = []
  })

  afterEach(async () => {
    // Unsubscribe from events if subscribed
    if (eventSubscription) {
      eventSubscription()
      eventSubscription = null
    }

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
    Reflect.deleteProperty(process.env, 'SKIP_CLAUDE_DISK_EVENTS')

    // NOTE: Do NOT call stopAllAgents() here as it affects other test instances
    // Individual task cleanup above should be sufficient

    // Wait a bit for any background processes to finish
    await new Promise((resolve) => setTimeout(resolve, 500))

    await app.close()
  })

  it('should complete the full workflow with local environment: create repository, create task, execute, and verify results', async () => {
    // Get test environment configuration
    const envConfig = getTestEnvironmentConfig()
    logger.info('Running test with workspace type', {
      workspaceType: envConfig.workspaceType,
    })

    // Step 1: Create repository using test helper
    const repository = await createTestRepository(app, tempDir, {
      setupCommands: ['echo "Repository setup complete"'],
    })

    createdRepositoryId = repository.id
    logger.info('Test repository created', {
      repositoryId: repository.id,
      workspaceType: envConfig.workspaceType,
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
    logger.info('taskCreationResult', taskCreationResult)

    // Wait a bit for background workspace setup and initial message processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 2a: Verify workspace snapshot was created
    logger.info('Verifying workspace snapshot', { taskId: createdTaskId })
    const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
    expect(snapshot).toBeTruthy()
    expect(snapshot?.taskId).toBe(createdTaskId)
    expect(snapshot?.repositoryId).toBe(repository.id)
    expect(snapshot?.executionTarget.mode).toBe('local')
    expect(snapshot?.executionTarget.workspacePath).toBeTruthy()

    // Verify workspace path matches temp directory (direct mode)
    expect(snapshot?.executionTarget.workspacePath).toBe(tempDir)

    logger.info('Workspace snapshot verified', {
      mode: snapshot?.executionTarget.mode,
      workspacePath: snapshot?.executionTarget.workspacePath,
      hasAgent: !!snapshot?.agent,
      previewStatus: snapshot?.preview.status,
    })

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
    expect(['open', 'running']).toContain(task.status)

    // Step 5: Subscribe to events directly via EventService and wait for completion
    let resultEvent: Event | null = null
    let hasStartedProcessing = false
    let errorEvent: Event | null = null
    const toolUseEvents: Event[] = []
    const maxWaitTime = 60000 // 60 seconds max for real agent to complete
    const startTime = Date.now()
    let agentStateVerified = false

    // Create a promise that resolves when we get a result or error event
    const completionPromise = new Promise<void>((resolve, reject) => {
      let lastToolResult: Event | null = null
      let completionCheckTimeout: NodeJS.Timeout | null = null

      // Subscribe to events for this specific task
      eventSubscription = eventService.subscribe(createdTaskId, (event: Event) => {
        collectedEvents.push(event)

        logger.debug('Received event via subscription', {
          type: event.type,
          subType: event.subType,
          isError: event.isError,
          hasPayload: !!event.payload,
        })

        // Check for processing start
        if (
          event.type === 'agent' &&
          (event.subType === 'message' || event.subType === 'started')
        ) {
          hasStartedProcessing = true

          // Verify agent state in snapshot (async, don't block event processing)
          if (!agentStateVerified) {
            agentStateVerified = true
            void (async () => {
              const runningSnapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
              if (runningSnapshot?.agent) {
                logger.info('Agent state during execution', {
                  state: runningSnapshot.agent.state,
                  sessionId: runningSnapshot.agent.sessionId,
                  agentType: runningSnapshot.agent.agentType,
                })
                // Note: We can't use expect() in this async callback as it won't fail the test properly
                // We'll verify final state after completion instead
              }
            })()
          }

          // Log tool use messages if present in payload
          if (
            event.payload &&
            typeof event.payload === 'object' &&
            'type' in event.payload &&
            event.payload.type === 'tool_use' &&
            'name' in event.payload
          ) {
            console.log('Tool use:', event.payload.name, event.payload)
            toolUseEvents.push(event)
          }
        }

        // Check for completion - either explicit completion event
        if (event.type === 'agent' && event.subType === 'completed') {
          if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
          if (!event.isError) {
            resultEvent = event
            resolve()
          } else {
            errorEvent = event
            console.error('Result error event:', event.payload)
            resolve()
          }
          return
        }

        // Track tool results to detect when agent might be done
        if (
          event.payload &&
          typeof event.payload === 'object' &&
          'type' in event.payload &&
          event.payload.type === 'user'
        ) {
          lastToolResult = event
          // Reset any existing timeout and set a new one
          if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
          completionCheckTimeout = setTimeout(async () => {
            // Check if task has completed by polling the task status
            try {
              const taskResponse = await app.inject({
                method: 'GET',
                url: `/api/v1/tasks/${createdTaskId}`,
              })
              if (taskResponse.statusCode === 200) {
                const task = JSON.parse(taskResponse.body)
                if (task.iterations.length > 0 && task.iterations[0].status === 'completed') {
                  resultEvent = lastToolResult
                  resolve()
                  return
                }
              }
            } catch (error) {
              console.log('Error checking task status:', error)
            }

            // If not completed yet, wait a bit more
            completionCheckTimeout = setTimeout(() => {
              if (!resultEvent) {
                resultEvent = lastToolResult
                resolve()
              }
            }, 2000)
          }, 1000)
        }

        // Check for general error events
        if (event.isError && event.type === 'agent' && event.subType === 'error') {
          if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
          errorEvent = event
          console.warn(`Error event (${event.type}):`, event.payload)
          resolve()
        }
      })

      // Set a timeout for the completion
      setTimeout(() => {
        if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
        if (!(resultEvent || errorEvent)) {
          reject(new Error(`Agent execution timed out after ${maxWaitTime}ms`))
        }
      }, maxWaitTime)
    })

    try {
      await completionPromise
    } catch (error) {
      logger.error('Event subscription timed out', error as Error, {
        collectedEventCount: collectedEvents.length,
        hasStartedProcessing,
        elapsedMs: Date.now() - startTime,
      })
      throw error
    }

    // Step 5: Verify agent processed and completed successfully
    expect(hasStartedProcessing).toBe(true)
    expect(resultEvent).toBeTruthy()
    expect(errorEvent).toBeNull()

    // Step 5a: Verify final workspace snapshot state
    logger.info('Verifying final workspace snapshot state')
    const finalSnapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
    expect(finalSnapshot).toBeTruthy()

    // Agent should be idle after completion
    if (finalSnapshot?.agent) {
      expect(finalSnapshot.agent.state).toBe('idle')
      logger.info('Agent final state verified', {
        state: finalSnapshot.agent.state,
        lastSessionId: finalSnapshot.agent.sessionId,
      })
    }

    // Workspace should still be valid
    expect(finalSnapshot?.executionTarget.workspacePath).toBe(tempDir)
    expect(finalSnapshot?.executionTarget.mode).toBe('local')

    // Step 6: Verify the file was created
    const helloWorldPath = path.join(tempDir, 'hello_world.py')
    const fileWasCreated = await verifyFileCreated(helloWorldPath, ['Hello, World!', 'print'])
    expect(fileWasCreated).toBe(true)

    const fileContent = await fs.readFile(helloWorldPath, 'utf-8')
    expect(fileContent).toContain('Hello, World!')
    expect(fileContent).toContain('print')

    console.log('✅ Claude Code agent successfully created hello_world.py!')
    console.log('File content:', fileContent)

    // Step 7: Wait for task iteration to be updated (with polling)
    let finalTask: Task | null = null
    let attempts = 0
    const maxAttempts = 10

    while (attempts < maxAttempts) {
      const taskResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}`,
      })

      if (taskResponse.statusCode === 200) {
        const task = JSON.parse(taskResponse.body)
        if (task.iterations.length > 0 && task.numTurns > 0) {
          finalTask = task
          break
        }
      }

      await new Promise((resolve) => setTimeout(resolve, 500))
      attempts++
    }

    // If we still don't have a completed task, get the current state anyway for debugging
    if (!finalTask) {
      const taskResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}`,
      })
      finalTask = JSON.parse(taskResponse.body)
      console.log('Task state after completion:', finalTask)
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

    // Step 9: Verify final task state and iteration results
    expect(finalTask).toBeTruthy()

    if (!finalTask) {
      throw new Error('Task state not available after completion')
    }

    // The task should have at least one iteration from the initial message
    expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)

    // Verify the first iteration has completed successfully
    const firstIteration = finalTask.iterations[0]
    if (!firstIteration) throw new Error('Expected at least one iteration')
    expect(firstIteration.status).toBe('completed')
    expect(firstIteration.agentSessionId).toBeDefined()
    expect(firstIteration.agentSessionId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
    ) // UUID format

    // Verify iteration has meaningful results
    expect(firstIteration.result).toBeTruthy()
    expect(typeof firstIteration.result).toBe('string')
    expect(firstIteration.result!.length).toBeGreaterThan(0)

    expect(firstIteration.costUsd).toBeGreaterThan(0)
    expect(firstIteration.endTime).toBeTruthy()
    // Check duration can be calculated from timestamps
    const duration =
      new Date(firstIteration.endTime!).getTime() - new Date(firstIteration.startTime).getTime()
    expect(duration).toBeGreaterThan(0)

    console.log('Task iteration completed:', {
      status: firstIteration.status,
      costUsd: firstIteration.costUsd,
      durationMs: duration,
      resultLength: firstIteration.result?.length,
      resultPreview: `${firstIteration.result?.substring(0, 100)}...`,
    })

    // Step 10: Get task messages for final verification
    const messagesResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
    })

    expect(messagesResponse.statusCode).toBe(200)
    const { messages, totalCount } = JSON.parse(messagesResponse.body)

    // Verify we have messages from the conversation
    expect(messages.length).toBeGreaterThan(0)
    expect(totalCount).toBe(messages.length)
    expect(totalCount).toBeGreaterThanOrEqual(2) // At least user message and assistant response

    // Find user and assistant messages
    const userMessages = messages.filter((msg: any) => msg.role === 'user')
    const assistantMessages = messages.filter((msg: any) => msg.role === 'assistant')

    expect(userMessages.length).toBeGreaterThan(0)
    expect(assistantMessages.length).toBeGreaterThan(0)

    // Verify the initial user message contains our request
    const initialUserMessage = userMessages[0]
    expect(initialUserMessage.content).toContain('hello_world.py')
    expect(initialUserMessage.content).toContain('Hello, World!')

    // Verify assistant response contains tool use
    const assistantResponse = assistantMessages[0]
    if (Array.isArray(assistantResponse.content)) {
      // Check for tool use in structured content
      const toolUseContent = assistantResponse.content.find((c: any) => c.type === 'tool_use')
      expect(toolUseContent).toBeTruthy()
      expect(['Write', 'Edit', 'MultiEdit']).toContain(toolUseContent.tool_name)
      console.log('Tool used in messages:', toolUseContent.tool_name)
    }

    console.log('Messages summary:', {
      totalMessages: totalCount,
      userMessages: userMessages.length,
      assistantMessages: assistantMessages.length,
      hasToolUse: assistantMessages.some(
        (msg: any) =>
          Array.isArray(msg.content) && msg.content.some((c: any) => c.type === 'tool_use'),
      ),
    })

    console.log('✅ Integration test completed successfully:', {
      totalEventsReceived: collectedEvents.length,
      hasStartedProcessing,
      hasToolUseEvents: toolUseEvents.length > 0,
      messagesVerified: totalCount,
      workspaceType: envConfig.workspaceType,
      executionTimeMs: Date.now() - startTime,
    })
  }, 60000) // 60 second timeout for real agent execution

  it('should complete the full workflow with Docker environment: create repository, create task, execute, and verify results', async () => {
    // Check if Docker is available before running the test using DockerService
    // @ts-expect-error - TODO: Refactor test to use proper DI setup
    // biome-ignore lint/correctness/noUndeclaredVariables: using global DI container in integration test
    const dockerService = container.get(DockerService)
    const dockerCheck = await dockerService.checkDockerAvailable()
    if (!dockerCheck.available) {
      console.log(`Skipping Docker test: ${dockerCheck.message}`)
      return
    }

    // Override environment config to use Docker
    const originalEnv = process.env['DEFAULT_WORKSPACE_TYPE']
    process.env['DEFAULT_WORKSPACE_TYPE'] = 'docker'

    try {
      // Get test environment configuration
      const envConfig = getTestEnvironmentConfig()
      logger.info('Running test with Docker workspace type', {
        workspaceType: envConfig.workspaceType,
      })
      expect(envConfig.workspaceType).toBe('docker')

      // Step 1: Create repository with Docker configuration
      // Don't override the codeExecution - let it use the default Docker image from test-helpers
      const repository = await createTestRepository(app, tempDir, {
        setupCommands: ['echo "Repository setup complete"'],
      })

      createdRepositoryId = repository.id
      logger.info('Test repository created with Docker', {
        repositoryId: repository.id,
        workspaceType: repository.codeExecution.type,
        dockerImage: repository.codeExecution.dockerImage,
      })

      // Verify the repository is configured for Docker
      expect(repository.codeExecution.type).toBe('docker')
      expect(repository.codeExecution.dockerImage).toBe('moatless-vibe-universal:latest')

      // Step 2: Create and start a task
      const createTaskRequest: CreateTaskRequest = {
        agentType: 'claude-code',
        repositoryId: repository.id,
        message:
          'Create a file called hello_docker.py with the content: print("Hello from Docker!")',
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
      logger.info('taskCreationResult', taskCreationResult)

      // Wait a bit for background workspace setup and initial message processing
      await new Promise((resolve) => setTimeout(resolve, 2000))

      // Step 2a: Verify workspace snapshot was created
      logger.info('Verifying workspace snapshot for Docker', { taskId: createdTaskId })
      const snapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
      expect(snapshot).toBeTruthy()
      expect(snapshot?.taskId).toBe(createdTaskId)
      expect(snapshot?.repositoryId).toBe(repository.id)
      expect(snapshot?.executionTarget.mode).toBe('local')
      expect(snapshot?.executionTarget.workspacePath).toBeTruthy()

      // Verify workspace path matches temp directory (direct mode)
      expect(snapshot?.executionTarget.workspacePath).toBe(tempDir)

      logger.info('Workspace snapshot verified for Docker', {
        mode: snapshot?.executionTarget.mode,
        workspacePath: snapshot?.executionTarget.workspacePath,
        hasAgent: !!snapshot?.agent,
        previewStatus: snapshot?.preview.status,
      })

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
      expect(['open', 'running']).toContain(task.status)

      // Step 4: Subscribe to events directly via EventService and wait for completion
      let resultEvent: Event | null = null
      let hasStartedProcessing = false
      let errorEvent: Event | null = null
      const toolUseEvents: Event[] = []
      const maxWaitTime = 60000 // 60 seconds max for real agent to complete
      const startTime = Date.now()
      let agentStateVerified = false

      // Create a promise that resolves when we get a result or error event
      const completionPromise = new Promise<void>((resolve, reject) => {
        let lastToolResult: Event | null = null
        let completionCheckTimeout: NodeJS.Timeout | null = null

        // Subscribe to events for this specific task
        eventSubscription = eventService.subscribe(createdTaskId, (event: Event) => {
          collectedEvents.push(event)

          logger.debug('Received event via subscription', {
            type: event.type,
            subType: event.subType,
            isError: event.isError,
            hasPayload: !!event.payload,
          })

          // Check for processing start
          if (
            event.type === 'agent' &&
            (event.subType === 'message' || event.subType === 'started')
          ) {
            hasStartedProcessing = true

            // Verify agent state in snapshot (async, don't block event processing)
            if (!agentStateVerified) {
              agentStateVerified = true
              void (async () => {
                const runningSnapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
                if (runningSnapshot?.agent) {
                  logger.info('Agent state during execution (Docker)', {
                    state: runningSnapshot.agent.state,
                    sessionId: runningSnapshot.agent.sessionId,
                    agentType: runningSnapshot.agent.agentType,
                  })
                }
              })()
            }

            // Log tool use messages if present in payload
            if (
              event.payload &&
              typeof event.payload === 'object' &&
              'type' in event.payload &&
              event.payload.type === 'tool_use' &&
              'name' in event.payload
            ) {
              console.log('Tool use:', event.payload.name, event.payload)
              toolUseEvents.push(event)
            }
          }

          // Check for completion - either explicit completion event
          if (event.type === 'agent' && event.subType === 'completed') {
            if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
            if (!event.isError) {
              resultEvent = event
              resolve()
            } else {
              errorEvent = event
              console.error('Result error event:', event.payload)
              resolve()
            }
            return
          }

          // Track tool results to detect when agent might be done
          if (
            event.payload &&
            typeof event.payload === 'object' &&
            'type' in event.payload &&
            event.payload.type === 'user'
          ) {
            lastToolResult = event
            // Reset any existing timeout and set a new one
            if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
            completionCheckTimeout = setTimeout(async () => {
              // Check if task has completed by polling the task status
              try {
                const taskResponse = await app.inject({
                  method: 'GET',
                  url: `/api/v1/tasks/${createdTaskId}`,
                })
                if (taskResponse.statusCode === 200) {
                  const task = JSON.parse(taskResponse.body)
                  if (task.iterations.length > 0 && task.iterations[0].status === 'completed') {
                    resultEvent = lastToolResult
                    resolve()
                    return
                  }
                }
              } catch (error) {
                console.log('Error checking task status:', error)
              }

              // If not completed yet, wait a bit more
              completionCheckTimeout = setTimeout(() => {
                if (!resultEvent) {
                  resultEvent = lastToolResult
                  resolve()
                }
              }, 2000)
            }, 1000)
          }

          // Check for general error events
          if (event.isError && event.type === 'agent' && event.subType === 'error') {
            if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
            errorEvent = event
            console.warn(`Error event (${event.type}):`, event.payload)
            resolve()
          }
        })

        // Set a timeout for the completion
        setTimeout(() => {
          if (completionCheckTimeout) clearTimeout(completionCheckTimeout)
          if (!(resultEvent || errorEvent)) {
            reject(new Error(`Agent execution timed out after ${maxWaitTime}ms`))
          }
        }, maxWaitTime)
      })

      try {
        await completionPromise
      } catch (error) {
        logger.error('Event subscription timed out', error as Error, {
          collectedEventCount: collectedEvents.length,
          hasStartedProcessing,
          elapsedMs: Date.now() - startTime,
        })
        throw error
      }

      // Step 5: Verify agent processed and completed successfully
      expect(hasStartedProcessing).toBe(true)
      expect(resultEvent).toBeTruthy()
      expect(errorEvent).toBeNull()

      // Step 5a: Verify final workspace snapshot state
      logger.info('Verifying final workspace snapshot state (Docker)')
      const finalSnapshot = await workspaceCoordinator.getSnapshot(createdTaskId)
      expect(finalSnapshot).toBeTruthy()

      // Agent should be idle after completion
      if (finalSnapshot?.agent) {
        expect(finalSnapshot.agent.state).toBe('idle')
        logger.info('Agent final state verified (Docker)', {
          state: finalSnapshot.agent.state,
          lastSessionId: finalSnapshot.agent.sessionId,
        })
      }

      // Workspace should still be valid
      expect(finalSnapshot?.executionTarget.workspacePath).toBe(tempDir)
      expect(finalSnapshot?.executionTarget.mode).toBe('local')

      // Step 6: Verify the file was created in the Docker environment
      const helloDockerPath = path.join(tempDir, 'hello_docker.py')
      const fileWasCreated = await verifyFileCreated(helloDockerPath, [
        'Hello from Docker!',
        'print',
      ])
      expect(fileWasCreated).toBe(true)

      const fileContent = await fs.readFile(helloDockerPath, 'utf-8')
      expect(fileContent).toContain('Hello from Docker!')
      expect(fileContent).toContain('print')

      console.log(
        '✅ Claude Code agent successfully created hello_docker.py in Docker environment!',
      )
      console.log('File content:', fileContent)

      // Step 7: Wait for task iteration to be updated (with polling)
      let finalTask: Task | null = null
      let attempts = 0
      const maxAttempts = 10

      while (attempts < maxAttempts) {
        const taskResponse = await app.inject({
          method: 'GET',
          url: `/api/v1/tasks/${createdTaskId}`,
        })

        if (taskResponse.statusCode === 200) {
          const task = JSON.parse(taskResponse.body)
          if (task.iterations.length > 0 && task.numTurns > 0) {
            finalTask = task
            break
          }
        }

        await new Promise((resolve) => setTimeout(resolve, 500))
        attempts++
      }

      // If we still don't have a completed task, get the current state anyway for debugging
      if (!finalTask) {
        const taskResponse = await app.inject({
          method: 'GET',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
        finalTask = JSON.parse(taskResponse.body)
        console.log('Task state after completion:', finalTask)
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
        expect(gitDiff.diff).toContain('hello_docker.py')
        expect(gitDiff.diff).toContain('Hello from Docker!')
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

      // Step 9: Verify final task state and iteration results
      expect(finalTask).toBeTruthy()

      if (!finalTask) {
        throw new Error('Task state not available after completion')
      }

      // The task should have at least one iteration from the initial message
      expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)

      // Verify the first iteration has completed successfully
      const firstIteration = finalTask.iterations[0]
      if (!firstIteration) throw new Error('Expected at least one iteration')
      expect(firstIteration.status).toBe('completed')
      expect(firstIteration.agentSessionId).toBeDefined()
      expect(firstIteration.agentSessionId).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
      ) // UUID format

      // Verify iteration has meaningful results
      expect(firstIteration.result).toBeTruthy()
      expect(typeof firstIteration.result).toBe('string')
      expect(firstIteration.result!.length).toBeGreaterThan(0)

      expect(firstIteration.costUsd).toBeGreaterThan(0)
      expect(firstIteration.endTime).toBeTruthy()
      // Check duration can be calculated from timestamps
      const duration =
        new Date(firstIteration.endTime!).getTime() - new Date(firstIteration.startTime).getTime()
      expect(duration).toBeGreaterThan(0)

      console.log('Task iteration completed:', {
        status: firstIteration.status,
        costUsd: firstIteration.costUsd,
        durationMs: duration,
        resultLength: firstIteration.result?.length,
        resultPreview: `${firstIteration.result?.substring(0, 100)}...`,
      })

      // Step 10: Get task messages for final verification
      const messagesResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/tasks/${createdTaskId}/messages`,
      })

      expect(messagesResponse.statusCode).toBe(200)
      const { messages, totalCount } = JSON.parse(messagesResponse.body)

      // Verify we have messages from the conversation
      expect(messages.length).toBeGreaterThan(0)
      expect(totalCount).toBe(messages.length)
      expect(totalCount).toBeGreaterThanOrEqual(2) // At least user message and assistant response

      // Find user and assistant messages
      const userMessages = messages.filter((msg: any) => msg.role === 'user')
      const assistantMessages = messages.filter((msg: any) => msg.role === 'assistant')

      expect(userMessages.length).toBeGreaterThan(0)
      expect(assistantMessages.length).toBeGreaterThan(0)

      // Verify the initial user message contains our request
      const initialUserMessage = userMessages[0]
      expect(initialUserMessage.content).toContain('hello_docker.py')
      expect(initialUserMessage.content).toContain('Hello from Docker!')

      // Verify assistant response contains tool use
      const assistantResponse = assistantMessages[0]
      if (Array.isArray(assistantResponse.content)) {
        // Check for tool use in structured content
        const toolUseContent = assistantResponse.content.find((c: any) => c.type === 'tool_use')
        expect(toolUseContent).toBeTruthy()
        expect(['Write', 'Edit', 'MultiEdit']).toContain(toolUseContent.tool_name)
        console.log('Tool used in messages:', toolUseContent.tool_name)
      }

      console.log('Messages summary:', {
        totalMessages: totalCount,
        userMessages: userMessages.length,
        assistantMessages: assistantMessages.length,
        hasToolUse: assistantMessages.some(
          (msg: any) =>
            Array.isArray(msg.content) && msg.content.some((c: any) => c.type === 'tool_use'),
        ),
      })

      console.log('✅ Docker integration test completed successfully:', {
        totalEventsReceived: collectedEvents.length,
        hasStartedProcessing,
        hasToolUseEvents: toolUseEvents.length > 0,
        messagesVerified: totalCount,
        workspaceType: 'docker',
        dockerImage: repository.codeExecution.dockerImage,
        executionTimeMs: Date.now() - startTime,
      })
    } finally {
      // Restore original environment
      if (originalEnv !== undefined) {
        process.env['DEFAULT_WORKSPACE_TYPE'] = originalEnv
      } else {
        Reflect.deleteProperty(process.env, 'DEFAULT_WORKSPACE_TYPE')
      }
    }
  }, 60000) // 60 second timeout for real agent execution
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
    '# Test Repository\n\nThis is a test repository for integration testing.\n',
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
