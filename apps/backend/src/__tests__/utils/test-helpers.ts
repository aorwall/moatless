/**
 * Common test utilities for integration tests
 * Provides helpers for testing with both local and Docker environments
 */

import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CreateRepositoryRequest,
  CreateTaskRequest,
  Event,
  RepositoryConfig,
  Task,
} from '@moatless/types'
import type { EventService } from '../../features/events/index'
import type { TestApp } from '../../test/helper'
import { logger } from '../../utils/logger'

export interface TestEnvironmentConfig {
  workspaceType: 'local' | 'docker'
  dockerImage?: string
  setupCommands?: string[]
  envVars?: Record<string, string>
}

export interface TaskExecutionResult {
  task: Task
  events: Event[]
  resultEvent: Event | null
  errorEvent: Event | null
  toolUseEvents: Event[]
  hasStartedProcessing: boolean
  executionTimeMs: number
}

/**
 * Creates a repository configuration for testing with the specified environment
 */
export function createTestRepositoryConfig(
  tempDir: string,
  config: TestEnvironmentConfig,
): CreateRepositoryRequest {
  return {
    name: `integration-test-repository-${config.workspaceType}`,
    description: `Test repository for ${config.workspaceType} environment validation`,
    path: tempDir,
    useGitWorktrees: false,
    cloneOnNewTask: false,
    codeExecution: {
      setupCommands: config.setupCommands || ['echo "Repository setup complete"'],
      envVars: config.envVars,
      dockerImage: config.dockerImage || 'moatless-vibe-universal:latest',
    },
  }
}

/**
 * Sets up a temporary git repository for testing
 */
export async function setupTempGitRepo(tempDir: string): Promise<void> {
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

/**
 * Runs a command in the specified directory
 */
export async function runCommand(command: string, args: string[], cwd: string): Promise<void> {
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

/**
 * Executes a task and waits for completion using direct EventService subscription
 */
export async function executeTaskWithEventSubscription(
  app: TestApp,
  eventService: EventService,
  taskId: string,
  message: string,
  timeoutMs = 60000,
): Promise<TaskExecutionResult> {
  const startTime = Date.now()
  const collectedEvents: Event[] = []
  let resultEvent: Event | null = null
  let hasStartedProcessing = false
  let errorEvent: Event | null = null
  const toolUseEvents: Event[] = []

  // Create task with message
  const createTaskResponse = await app.inject({
    method: 'POST',
    url: '/api/v1/tasks',
    payload: {
      agentType: 'claude-code',
      repositoryId: taskId, // This should be repository ID in real usage
      message,
      start: true,
    } as CreateTaskRequest,
  })

  if (createTaskResponse.statusCode !== 200) {
    throw new Error(`Failed to create task: ${createTaskResponse.body}`)
  }

  const taskResult = JSON.parse(createTaskResponse.body)
  const createdTaskId = taskResult.taskId

  // Create a promise that resolves when we get a result or error event
  const completionPromise = new Promise<void>((resolve, reject) => {
    let eventSubscription: (() => void) | null = null

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
      if (event.type === 'agent' && (event.subType === 'message' || event.subType === 'started')) {
        hasStartedProcessing = true

        // Log tool use messages if present in payload
        if (
          event.payload &&
          typeof event.payload === 'object' &&
          'type' in event.payload &&
          event.payload.type === 'tool_use' &&
          'name' in event.payload
        ) {
          logger.info('Tool use detected', {
            name: event.payload.name,
            payload: event.payload,
          })
          toolUseEvents.push(event)
        }
      }

      // Check for completion
      if (event.type === 'agent' && event.subType === 'completed') {
        if (!event.isError) {
          resultEvent = event
          eventSubscription?.()
          resolve()
        } else {
          errorEvent = event
          logger.error('Result error event:', event.payload)
          eventSubscription?.()
          resolve() // Still resolve, we'll check for errors later
        }
      }

      // Check for general error events
      if (event.isError && event.type === 'agent' && event.subType === 'error') {
        errorEvent = event
        logger.warn(`Error event (${event.type}):`, event.payload)
        eventSubscription?.()
        resolve()
      }
    })

    // Set a timeout for the completion
    const timeoutId = setTimeout(() => {
      if (!(resultEvent || errorEvent)) {
        eventSubscription?.()
        reject(new Error(`Agent execution timed out after ${timeoutMs}ms`))
      }
    }, timeoutMs)

    // Clear timeout when resolved
    completionPromise.finally(() => clearTimeout(timeoutId))
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

  // Get final task state
  const finalTaskResponse = await app.inject({
    method: 'GET',
    url: `/api/v1/tasks/${createdTaskId}`,
  })

  if (finalTaskResponse.statusCode !== 200) {
    throw new Error(`Failed to get final task state: ${finalTaskResponse.body}`)
  }

  const finalTask: Task = JSON.parse(finalTaskResponse.body)

  return {
    task: finalTask,
    events: collectedEvents,
    resultEvent,
    errorEvent,
    toolUseEvents,
    hasStartedProcessing,
    executionTimeMs: Date.now() - startTime,
  }
}

/**
 * Gets the environment configuration from environment variables or defaults to local
 */
export function getTestEnvironmentConfig(): TestEnvironmentConfig {
  const workspaceType = process.env['DEFAULT_WORKSPACE_TYPE'] === 'docker' ? 'docker' : 'local'

  return {
    workspaceType,
    dockerImage: process.env['TEST_DOCKER_IMAGE'] || 'moatless-vibe-universal:latest',
    setupCommands: ['echo "Repository setup complete"'],
  }
}

/**
 * Creates a repository for testing with the current environment configuration
 */
export async function createTestRepository(
  app: TestApp,
  tempDir: string,
  config?: Partial<TestEnvironmentConfig>,
): Promise<RepositoryConfig> {
  const envConfig = getTestEnvironmentConfig()
  const finalConfig = { ...envConfig, ...config }

  const createRepositoryRequest = createTestRepositoryConfig(tempDir, finalConfig)

  const createRepoResponse = await app.inject({
    method: 'POST',
    url: '/api/v1/repositories',
    payload: createRepositoryRequest,
  })

  if (createRepoResponse.statusCode !== 200) {
    throw new Error(`Failed to create repository: ${createRepoResponse.body}`)
  }

  const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
  logger.info('Test repository created', {
    repositoryId: repository.id,
    workspaceType: finalConfig.workspaceType,
  })

  return repository
}

/**
 * Verifies that a file was created with expected content
 */
export async function verifyFileCreated(
  filePath: string,
  expectedContent: string | string[],
): Promise<boolean> {
  try {
    const fileContent = await fs.readFile(filePath, 'utf-8')

    if (Array.isArray(expectedContent)) {
      return expectedContent.every((content) => fileContent.includes(content))
    }

    return fileContent.includes(expectedContent)
  } catch {
    return false
  }
}

/**
 * Cleanup helper for test resources
 */
export async function cleanupTestResources(
  app: TestApp,
  options: {
    taskId?: string
    repositoryId?: string
    tempDir?: string
  },
): Promise<void> {
  const { taskId, repositoryId, tempDir } = options

  // Stop and cleanup task
  if (taskId) {
    try {
      await app.inject({
        method: 'POST',
        url: `/api/v1/tasks/${taskId}/stop`,
      })
      await new Promise((resolve) => setTimeout(resolve, 100))
    } catch {
      // Agent might not be running
    }

    try {
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/tasks/${taskId}`,
      })
    } catch {
      // Task might already be deleted
    }
  }

  // Cleanup repository
  if (repositoryId) {
    try {
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/repositories/${repositoryId}`,
      })
    } catch {
      // Repository might already be deleted
    }
  }

  // Clean up temp directory
  if (tempDir) {
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch {
      // Directory might already be deleted
    }
  }
}
