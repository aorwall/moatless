import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateTaskRequest, Event, Task } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../features/events/index'
import type { WorkspaceCoordinator } from '../features/workspace/index'
import { build } from '../test/helper'
import { getDataDir } from '../utils/data-dir'
import { logger } from '../utils/logger'
import {
  createTestRepository,
  getTestEnvironmentConfig,
  verifyFileCreated,
} from './utils/test-helpers'

/**
 * Integration test for file upload and agent file operations
 *
 * This test verifies that:
 * 1. Files can be uploaded to the uploads directory
 * 2. The agent can access uploaded files via the allowed directories
 * 3. The agent can copy file content to the repository
 *
 * Prerequisites:
 * 1. Run "claude login" to authenticate before running this test
 * 2. Ensure Claude Code CLI is properly configured
 */

describe('File Upload Workflow Integration Test (broad)', () => {
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
    tempDir = await fs.mkdtemp(path.join(dataDir, 'moatless-upload-test-'))

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

    // Clean up test environment variables
    Reflect.deleteProperty(process.env, 'SKIP_CLAUDE_DISK_EVENTS')
    Reflect.deleteProperty(process.env, 'DATA_DIR')

    // Wait a bit for any background processes to finish
    await new Promise((resolve) => setTimeout(resolve, 500))

    await app.close()
  })

  it('should upload a file and have agent copy it to the repository', async () => {
    // Get test environment configuration
    const envConfig = getTestEnvironmentConfig()
    logger.info(
      {
        workspaceType: envConfig.workspaceType,
      },
      'Running file upload test with workspace type',
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

    // Step 2: Upload a test file
    const testContent =
      'This is test content that should be copied to the repository.\nLine 2 of the test file.\nLine 3 with special chars: !@#$%'
    const testFilename = 'source-file.txt'

    const formData = new FormData()
    const file = new File([testContent], testFilename, { type: 'text/plain' })
    formData.append('file', file)

    const uploadResponse = await app.fetch(
      new Request('http://localhost/api/v1/uploads', {
        method: 'POST',
        body: formData,
      }),
    )

    expect(uploadResponse.status).toBe(200)
    const uploadedFile = (await uploadResponse.json()) as {
      fileId: string
      filename: string
      mimeType: string
      size: number
    }

    expect(uploadedFile.fileId).toBeDefined()
    expect(uploadedFile.filename).toBe(testFilename)
    expect(uploadedFile.mimeType).toContain('text/plain')

    logger.info(
      {
        fileId: uploadedFile.fileId,
        filename: uploadedFile.filename,
        size: uploadedFile.size,
      },
      'File uploaded successfully',
    )

    // Step 3: Create and start a task with the uploaded file
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: `I have attached a file called "${testFilename}". The file is available in the uploads directory and you can access it.

Please create a new file in the repository root directory called "copied-file.txt" with the EXACT same content as the attached file.

The content should be:
- Line 1: "This is test content that should be copied to the repository."
- Line 2: "Line 2 of the test file."
- Line 3: "Line 3 with special chars: !@#$%"

Make sure to preserve all content exactly as shown above.`,
      fileIds: [uploadedFile.fileId],
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
      'Task created with file attachment',
    )

    // Wait a bit for background workspace setup and initial message processing
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Step 4: Verify task was created with file attachment
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

    // Step 5: Subscribe to events and wait for completion
    let resultEvent: Event | null = null
    let hasStartedProcessing = false
    let errorEvent: Event | null = null
    const maxWaitTime = 60000 // 60 seconds max for real agent to complete
    const startTime = Date.now()

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

    // Step 6: Verify agent processed and completed successfully
    expect(hasStartedProcessing).toBe(true)
    expect(resultEvent).toBeTruthy()
    expect(errorEvent).toBeNull()

    // Step 7: Verify the file was copied to the repository
    const copiedFilePath = path.join(tempDir, 'copied-file.txt')
    const fileWasCreated = await verifyFileCreated(copiedFilePath, [
      'test content',
      'Line 2',
      'Line 3',
    ])
    expect(fileWasCreated).toBe(true)

    const fileContent = await fs.readFile(copiedFilePath, 'utf-8')
    expect(fileContent).toContain('This is test content that should be copied to the repository.')
    expect(fileContent).toContain('Line 2 of the test file.')
    expect(fileContent).toContain('Line 3 with special chars: !@#$%')

    console.log('File was successfully copied to repository!')
    console.log('Copied file content:', fileContent)

    // Step 8: Verify task messages contain the attachment
    const messagesResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}/messages`,
    })

    expect(messagesResponse.statusCode).toBe(200)
    const { messages, totalCount } = JSON.parse(messagesResponse.body)

    // Verify we have messages from the conversation
    expect(messages.length).toBeGreaterThan(0)
    expect(totalCount).toBe(messages.length)

    // Find user messages and verify attachment
    const userMessages = messages.filter((msg: any) => msg.role === 'user')
    expect(userMessages.length).toBeGreaterThan(0)

    // The first user message should have the attachment
    const initialUserMessage = userMessages[0]
    if (initialUserMessage.attachments && initialUserMessage.attachments.length > 0) {
      const attachment = initialUserMessage.attachments[0]
      expect(attachment.filename).toBe(testFilename)
      expect(attachment.mimeType).toBe('text/plain')
      console.log('Verified attachment in message:', attachment)
    }

    // Step 9: Verify final task state
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

    expect(finalTask).toBeTruthy()
    if (finalTask) {
      expect(finalTask.iterations.length).toBeGreaterThanOrEqual(1)
      const firstIteration = finalTask.iterations[0]
      if (firstIteration) {
        expect(firstIteration.status).toBe('completed')
        expect(firstIteration.agentSessionId).toBeDefined()
      }
    }

    console.log('File upload workflow test completed successfully:', {
      totalEventsReceived: collectedEvents.length,
      hasStartedProcessing,
      executionTimeMs: Date.now() - startTime,
      fileUploaded: testFilename,
      fileCopied: 'copied-file.txt',
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
    '# Test Repository\n\nThis is a test repository for file upload integration testing.\n',
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
