import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import type {
  CreateRepositoryRequest,
  RepositoryConfig,
  SetupScriptResponse,
} from '@moatless/shared/types/index'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'
import { logger } from '../../utils/logger.js'

const execAsync = promisify(exec)

/**
 * Docker Workspace Setup Commands Integration Test
 *
 * This test validates that Docker workspaces can properly execute setup commands
 * like 'pnpm install' using the hello-world-ui repository as a test case.
 *
 * Prerequisites:
 * - Docker must be installed and running
 * - Git must be installed
 * - The moatless-backend:latest Docker image must be built
 */

describe('Docker Workspace Setup Commands Test', () => {
  let app: any
  let tempDir: string
  let createdRepositoryId: string

  beforeEach(async () => {
    // Create a temporary directory for our test repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-setup-test-'))
    logger.info('Created temp directory:', { tempDir })

    // Clone the hello-world-ui repository
    logger.info('Cloning hello-world-ui repository...')
    await execAsync(`git clone https://github.com/aorwall/hello-world-ui.git "${tempDir}"`, {
      timeout: 30000,
    })
    logger.info('Repository cloned successfully')

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    app = await build()

    // Start the server on a random available port
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Get the actual port the server is listening on
    const actualPort = app.server?.address().port
    if (actualPort) {
      // Set environment variable for Docker workspace to use
      process.env.WORKER_API_URL = `http://host.docker.internal:${actualPort}`
      process.env.WORKER_API_KEY = 'test-key'
    }
  })

  afterEach(async () => {
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

    // Clean up temp directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (_error) {
      // Directory might already be deleted
    }

    // Clean up environment variables
    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    delete process.env.WORKER_API_URL
    delete process.env.WORKER_API_KEY

    await app.close()
  }, 30000) // 30 second timeout for cleanup

  it('should execute basic commands in Docker workspace', async () => {
    // Step 1: Create repository with Docker workspace type
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker Basic Commands Test',
      description: 'Test repository for basic Docker workspace commands',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        workspaceType: 'docker',
        setupCommands: 'echo "Hello from Docker"\nls -la\npwd\nnode --version', // Basic commands
        image: 'node:18',
        volumeMapping: {
          hostPath: tempDir,
          containerPath: '/workspace',
        },
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

    // Step 2: Run setup script
    logger.info('Running basic commands in Docker workspace...')
    const setupResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/repositories/${repository.id}/run-setup`,
    })

    expect(setupResponse.statusCode).toBe(200)
    const setupResult: SetupScriptResponse = JSON.parse(setupResponse.body)

    logger.info('Setup script result:', {
      success: setupResult.success,
      commandCount: setupResult.commands.length,
    })

    // Log all commands
    setupResult.commands.forEach((cmd, index) => {
      logger.info(`Command ${index + 1}:`, {
        command: cmd.command,
        success: cmd.success,
        exitCode: cmd.exit_code,
        output: cmd.output,
        error: cmd.error,
      })
    })

    // Verify all commands succeeded
    expect(setupResult.commands).toHaveLength(4)

    // Check echo command
    expect(setupResult.commands[0]).toMatchObject({
      command: 'echo "Hello from Docker"',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[0].output).toContain('Hello from Docker')

    // Check ls command
    expect(setupResult.commands[1]).toMatchObject({
      command: 'ls -la',
      success: true,
      exit_code: 0,
    })

    // Check pwd command
    expect(setupResult.commands[2]).toMatchObject({
      command: 'pwd',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[2].output).toContain('/workspace')

    // Check node version
    expect(setupResult.commands[3]).toMatchObject({
      command: 'node --version',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[3].output).toMatch(/v\d+\.\d+\.\d+/)
  }, 60000)

  it('should execute file operations in Docker workspace', async () => {
    // Step 1: Create repository with Docker workspace type
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker Setup Test Repository',
      description: 'Test repository for Docker workspace setup commands',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        workspaceType: 'docker', // Use Docker workspace
        setupCommands: 'echo "Test content" > test.txt\ncat test.txt\nls -la test.txt', // Simple file creation
        image: 'node:18', // Use Node.js image
        volumeMapping: {
          hostPath: tempDir,
          containerPath: '/workspace',
        },
      },
    }

    const createRepoResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)
    logger.info('Repository created:', {
      id: repository.id,
      name: repository.name,
      workspaceType: repository.codeExecution.workspaceType,
    })

    createdRepositoryId = repository.id

    expect(repository).toMatchObject({
      name: createRepositoryRequest.name,
      description: createRepositoryRequest.description,
      path: createRepositoryRequest.path,
      codeExecution: expect.objectContaining({
        type: 'local',
        workspaceType: 'docker',
        setupCommands: 'echo "Test content" > test.txt\ncat test.txt\nls -la test.txt',
        image: 'node:18',
      }),
    })

    // Step 2: Run setup script (which should execute in Docker)
    logger.info('Running setup script in Docker workspace...')
    const setupResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/repositories/${repository.id}/run-setup`,
    })

    expect(setupResponse.statusCode).toBe(200)
    const setupResult: SetupScriptResponse = JSON.parse(setupResponse.body)
    logger.info('Setup script result:', {
      success: setupResult.success,
      commandCount: setupResult.commands.length,
    })

    // Log command details for debugging
    setupResult.commands.forEach((cmd, index) => {
      logger.info(`Command ${index + 1}:`, {
        command: cmd.command,
        success: cmd.success,
        exitCode: cmd.exit_code,
        output: cmd.output || '(no output)',
        error: cmd.error || '(no error)',
      })
    })

    // Verify setup succeeded
    expect(setupResult.success).toBe(true)
    expect(setupResult.commands).toHaveLength(3)

    // Check echo command
    expect(setupResult.commands[0]).toMatchObject({
      command: 'echo "Test content" > test.txt',
      success: true,
      exit_code: 0,
    })

    // Check cat command
    expect(setupResult.commands[1]).toMatchObject({
      command: 'cat test.txt',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[1].output).toContain('Test content')

    // Check ls command
    expect(setupResult.commands[2]).toMatchObject({
      command: 'ls -la test.txt',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[2].output).toMatch(/test\.txt/)

    // Step 3: Verify that test.txt was created in the host directory
    const testFilePath = path.join(tempDir, 'test.txt')
    const testFileExists = await fs
      .access(testFilePath)
      .then(() => true)
      .catch(() => false)

    expect(testFileExists).toBe(true)
    logger.info('Verified test.txt was created in host volume')

    // Read and verify file content
    const fileContent = await fs.readFile(testFilePath, 'utf-8')
    expect(fileContent.trim()).toBe('Test content')
  }, 120000) // 2 minute timeout for Docker operations

  it('should handle Docker setup with multiple commands', async () => {
    // Test with multiple setup commands
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker Multi-Command Test',
      description: 'Test repository for multiple Docker setup commands',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        workspaceType: 'docker',
        setupCommands: 'echo "Starting setup"\nnpm install\necho "Setup complete"',
        image: 'node:20-slim',
        volumeMapping: {
          hostPath: tempDir,
          containerPath: '/workspace',
        },
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

    // Run setup script
    const setupResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/repositories/${repository.id}/run-setup`,
    })

    expect(setupResponse.statusCode).toBe(200)
    const setupResult: SetupScriptResponse = JSON.parse(setupResponse.body)

    // Verify all commands succeeded
    expect(setupResult.success).toBe(true)
    expect(setupResult.commands).toHaveLength(3)

    // Check first echo command
    expect(setupResult.commands[0]).toMatchObject({
      command: 'echo "Starting setup"',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[0].output).toContain('Starting setup')

    // Check pnpm install
    expect(setupResult.commands[1]).toMatchObject({
      command: 'npm install',
      success: true,
      exit_code: 0,
    })

    // Check second echo command
    expect(setupResult.commands[2]).toMatchObject({
      command: 'echo "Setup complete"',
      success: true,
      exit_code: 0,
    })
    expect(setupResult.commands[2].output).toContain('Setup complete')
  }, 120000)

  it('should handle Docker setup command failures gracefully', async () => {
    // Test with a command that will fail
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Docker Failure Test',
      description: 'Test repository for Docker setup command failures',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        workspaceType: 'docker',
        setupCommands: 'echo "Starting setup"\nfalse\necho "This should not run"',
        image: 'node:20-slim',
        volumeMapping: {
          hostPath: tempDir,
          containerPath: '/workspace',
        },
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

    // Run setup script
    const setupResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/repositories/${repository.id}/run-setup`,
    })

    expect(setupResponse.statusCode).toBe(200)
    const setupResult: SetupScriptResponse = JSON.parse(setupResponse.body)

    // Verify overall failure but partial execution
    expect(setupResult.success).toBe(false)
    expect(setupResult.commands).toHaveLength(3)

    // First command should succeed
    expect(setupResult.commands[0]).toMatchObject({
      command: 'echo "Starting setup"',
      success: true,
      exit_code: 0,
    })

    // Second command should fail
    expect(setupResult.commands[1]).toMatchObject({
      command: 'false',
      success: false,
      exit_code: 1,
    })

    // Third command should still execute (we continue on failure)
    expect(setupResult.commands[2]).toBeDefined()
  }, 60000)
})
