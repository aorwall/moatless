import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateTaskRequest, Event, Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../features/events/index'
import type { WorkspaceRuntimeManager } from '../features/workspace/index'
import { getWorkspacePathFromTarget } from '../features/workspace/types/workspace-runtime.types'
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

describe.skipIf(!process.env['DATABASE_URL'])('Complete Workflow Integration Test (broad)', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let eventService: EventService
  let workspaceCoordinator: WorkspaceRuntimeManager
  let eventSubscription: (() => void) | null = null
  let collectedEvents: Event[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test git repository within the data directory
    const dataDir = getDataDir()
    tempDir = await fs.mkdtemp(path.join(dataDir, 'moatless-e2e-test-'))

    // Set DATA_DIR to the temp directory so storage uses it
    process.env['DATA_DIR'] = tempDir

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
      throw new Error('WorkspaceRuntimeManager not available in test app')
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

    // Clean up test environment variables
    Reflect.deleteProperty(process.env, 'SKIP_CLAUDE_DISK_EVENTS')
    Reflect.deleteProperty(process.env, 'DATA_DIR')

    // NOTE: Do NOT call stopAllAgents() here as it affects other test instances
    // Individual task cleanup above should be sufficient

    // Wait a bit for any background processes to finish
    await new Promise((resolve) => setTimeout(resolve, 500))

    await app.close()
  })

  it('should complete the full workflow with local environment: create repository, create task, execute, and verify results', async () => {
    // Get test environment configuration
    const envConfig = getTestEnvironmentConfig()
    logger.info(
      {
        workspaceType: envConfig.workspaceType,
      },
      'Running test with workspace type',
    )

    // Step 1: Create repository using test helper
    const repository = await createTestRepository(app, tempDir, {
      setupCommands: ['echo "Repository setup complete"'],
    })

    createdRepositoryId = repository.id
    logger.info(
      {
        repositoryId: repository.id,
        workspaceType: envConfig.workspaceType,
      },
      'Test repository created',
    )

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

    if (createTaskResponse.statusCode !== 200) {
      logger.error(
        {
          statusCode: createTaskResponse.statusCode,
          body: createTaskResponse.body,
        },
        'Task creation failed',
      )
    }
    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId
    expect(taskCreationResult.status).toBe('open')
    logger.info(
      { taskId: createdTaskId, status: taskCreationResult.status },
      'Task created successfully',
    )

    // Wait a bit for background workspace setup and initial message processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 2a: Verify workspace sandbox record was created
    logger.info({ taskId: createdTaskId }, 'Verifying workspace sandbox record')
    const sandboxRecord = await workspaceCoordinator.getSandboxRecord(createdTaskId)
    expect(sandboxRecord).toBeTruthy()
    expect(sandboxRecord?.taskId).toBe(createdTaskId)
    expect(sandboxRecord?.executionTarget.mode).toBe('local')

    // Use type guard to safely access local execution target properties
    const workspacePath = sandboxRecord
      ? getWorkspacePathFromTarget(sandboxRecord.executionTarget)
      : undefined
    expect(workspacePath).toBeTruthy()

    // Verify workspace path matches temp directory (direct mode)
    expect(workspacePath).toBe(tempDir)

    logger.info(
      {
        mode: sandboxRecord?.executionTarget.mode,
        workspacePath,
      },
      'Workspace sandbox record verified',
    )

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

        logger.debug(
          {
            type: event.type,
            subType: event.subType,
            isError: event.isError,
            hasPayload: !!event.payload,
          },
          'Received event via subscription',
        )

        // Check for processing start
        if (
          event.type === 'agent' &&
          (event.subType === 'message' || event.subType === 'started')
        ) {
          hasStartedProcessing = true

          // Verify agent is running (async, don't block event processing)
          if (!agentStateVerified) {
            agentStateVerified = true
            void (async () => {
              const isRunning = await workspaceCoordinator.isAgentActuallyRunning(createdTaskId)
              logger.info({ isRunning }, 'Agent running state during execution')
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
                if (task.status === 'open') {
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
      logger.error(
        {
          err: error as Error,
          collectedEventCount: collectedEvents.length,
          hasStartedProcessing,
          elapsedMs: Date.now() - startTime,
        },
        'Event subscription timed out',
      )
      throw error
    }

    // Step 5: Verify agent processed and completed successfully
    expect(hasStartedProcessing).toBe(true)
    expect(resultEvent).toBeTruthy()
    expect(errorEvent).toBeNull()

    // Step 5a: Verify final workspace state
    logger.info('Verifying final workspace state')

    // Wait for agent to become idle (agent state is ephemeral — queried live)
    const pollStartTime = Date.now()
    const pollTimeout = 5000 // 5 seconds max
    const pollInterval = 100 // Check every 100ms

    while (Date.now() - pollStartTime < pollTimeout) {
      const isRunning = await workspaceCoordinator.isAgentActuallyRunning(createdTaskId)
      if (!isRunning) {
        break
      }
      await new Promise((resolve) => setTimeout(resolve, pollInterval))
    }

    const isStillRunning = await workspaceCoordinator.isAgentActuallyRunning(createdTaskId)
    expect(isStillRunning).toBe(false)
    logger.info({ waitTime: Date.now() - pollStartTime }, 'Agent final state verified (idle)')

    // Workspace should still be valid
    const finalRecord = await workspaceCoordinator.getSandboxRecord(createdTaskId)
    expect(finalRecord).toBeTruthy()
    const finalWorkspacePath = finalRecord
      ? getWorkspacePathFromTarget(finalRecord.executionTarget)
      : undefined
    expect(finalWorkspacePath).toBe(tempDir)
    expect(finalRecord?.executionTarget.mode).toBe('local')

    // Step 6: Verify the file was created
    const helloWorldPath = path.join(tempDir, 'hello_world.py')
    const fileWasCreated = await verifyFileCreated(helloWorldPath, ['Hello, World!', 'print'])
    expect(fileWasCreated).toBe(true)

    const fileContent = await fs.readFile(helloWorldPath, 'utf-8')
    expect(fileContent).toContain('Hello, World!')
    expect(fileContent).toContain('print')

    console.log('✅ Claude Code agent successfully created hello_world.py!')
    console.log('File content:', fileContent)

    // Step 6.5: Read file via FileGateway API route
    const readFileResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/files/hello_world.py`,
    })

    if (readFileResponse.statusCode !== 200) {
      logger.error('Read file via API failed')
    }
    expect(readFileResponse.statusCode).toBe(200)
    const fileReadData = JSON.parse(readFileResponse.body)

    // Verify file content matches what we read from disk
    expect(fileReadData.content).toBe(fileContent)
    expect(fileReadData.encoding).toBe('utf-8')

    // Verify git change information is included
    // Note: isModified may be false for newly created unstaged files
    // The important thing is we get the git change data
    expect(fileReadData).toHaveProperty('isModified')
    expect(fileReadData).toHaveProperty('additions')
    expect(fileReadData).toHaveProperty('deletions')

    console.log('✅ File read via FileGateway API successful:', {
      contentLength: fileReadData.content.length,
      isModified: fileReadData.isModified,
      additions: fileReadData.additions,
      deletions: fileReadData.deletions,
    })

    // Step 6.75: List files via FileGateway API route
    const listFilesResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/files?recursive=true`,
    })

    if (listFilesResponse.statusCode !== 200) {
      logger.error(
        {
          statusCode: listFilesResponse.statusCode,
          body: listFilesResponse.body,
        },
        'List files via API failed',
      )
    }
    expect(listFilesResponse.statusCode).toBe(200)
    const fileListData = JSON.parse(listFilesResponse.body)

    // Verify response structure
    expect(fileListData.files).toBeDefined()
    expect(Array.isArray(fileListData.files)).toBe(true)
    expect(fileListData.totalCount).toBe(fileListData.files.length)

    // Verify hello_world.py is in the list (match by filename, not full path)
    const helloWorldFile = fileListData.files.find(
      (f: any) => f.path === 'hello_world.py' || f.path.endsWith('/hello_world.py'),
    )
    if (!helloWorldFile) {
      console.log(
        'Files found:',
        fileListData.files.map((f: any) => f.path),
      )
    }
    expect(helloWorldFile).toBeDefined()
    expect(helloWorldFile.type).toBe('file')

    // Verify README.md is also present (initial file)
    const readmeFile = fileListData.files.find(
      (f: any) => f.path === 'README.md' || f.path.endsWith('/README.md'),
    )
    expect(readmeFile).toBeDefined()

    console.log('✅ File list via FileGateway API successful:', {
      totalFiles: fileListData.totalCount,
      files: fileListData.files.map((f: any) => f.path),
    })

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
        if (task.status !== 'running') {
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

    // Step 8: Verify final task state and iteration results
    expect(finalTask).toBeTruthy()

    if (!finalTask) {
      throw new Error('Task state not available after completion')
    }

    // Verify the task completed successfully (status is 'open' after successful turn)
    expect(['open', 'closed', 'merged']).toContain(finalTask.status)

    // Verify task has accumulated cost
    expect(finalTask.totalCostUsd).toBeGreaterThan(0)

    console.log('Task turn completed:', {
      status: finalTask.status,
      totalCostUsd: finalTask.totalCostUsd,
    })

    // Step 10: Get task messages for final verification
    const messagesResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
    })

    if (messagesResponse.statusCode !== 200) {
      logger.error(
        {
          statusCode: messagesResponse.statusCode,
          body: messagesResponse.body,
        },
        'Get messages failed',
      )
    }
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
    const userContent =
      typeof initialUserMessage.content === 'string'
        ? initialUserMessage.content
        : JSON.stringify(initialUserMessage.content)
    expect(userContent).toContain('hello_world.py')
    expect(userContent).toContain('Hello, World!')

    // Verify assistant response exists and contains content
    const assistantResponse = assistantMessages[0]
    expect(assistantResponse).toBeTruthy()
    expect(assistantResponse.content).toBeTruthy()

    // Check for tool use in messages (optional - depends on message format)
    let hasToolUse = false
    let toolName: string | undefined

    if (Array.isArray(assistantResponse.content)) {
      // Check for tool use in structured content
      const toolUseContent = assistantResponse.content.find((c: any) => c.type === 'tool_use')
      if (toolUseContent) {
        hasToolUse = true
        toolName = toolUseContent.tool_name
        console.log('Tool used in messages:', toolName)
        expect(['Write', 'Edit', 'MultiEdit']).toContain(toolName)
      }
    }

    // If no tool use in content, check if any assistant message has tool use
    if (!hasToolUse) {
      hasToolUse = assistantMessages.some(
        (msg: any) =>
          Array.isArray(msg.content) && msg.content.some((c: any) => c.type === 'tool_use'),
      )
    }

    console.log('Messages summary:', {
      totalMessages: totalCount,
      userMessages: userMessages.length,
      assistantMessages: assistantMessages.length,
      hasToolUse,
      toolName,
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
