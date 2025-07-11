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

describe('Docker Setup Minimal Test', () => {
  let app: any
  let tempDir: string

  beforeEach(async () => {
    // Create temp directory and init git repo
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-minimal-test-'))
    await execAsync(`cd "${tempDir}" && git init`)
    logger.info('Created temp git repo:', { tempDir })

    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'
    app = await build()
    await app.listen({ port: 0, host: '127.0.0.1' })

    const actualPort = app.server?.address().port
    if (actualPort) {
      process.env.WORKER_API_URL = `http://host.docker.internal:${actualPort}`
      process.env.WORKER_API_KEY = 'test-key'
    }
  })

  afterEach(async () => {
    // Clean up temp directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (_error) {
      // Directory might already be deleted
    }

    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    delete process.env.WORKER_API_URL
    delete process.env.WORKER_API_KEY
    await app.close()
  })

  it('should execute echo command in Docker', async () => {
    // Create repository with minimal Docker config
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Minimal Docker Test',
      description: 'Test echo command',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
        workspaceType: 'docker',
        setupCommands: 'echo "Hello Docker"',
        image: 'alpine:latest',
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

    if (createRepoResponse.statusCode !== 200) {
      logger.error('Failed to create repository:', {
        status: createRepoResponse.statusCode,
        body: createRepoResponse.body,
      })
    }

    expect(createRepoResponse.statusCode).toBe(200)
    const repository: RepositoryConfig = JSON.parse(createRepoResponse.body)

    logger.info('Created repository:', { id: repository.id })

    // Run setup
    const setupResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/repositories/${repository.id}/run-setup`,
    })

    logger.info('Setup response status:', setupResponse.statusCode)
    logger.info('Setup response body:', setupResponse.body)

    expect(setupResponse.statusCode).toBe(200)
    const setupResult: SetupScriptResponse = JSON.parse(setupResponse.body)

    logger.info('Setup result:', {
      success: setupResult.success,
      commandCount: setupResult.commands.length,
      commands: setupResult.commands,
    })

    // Verify command succeeded
    expect(setupResult.success).toBe(true)
    expect(setupResult.commands).toHaveLength(1)
    expect(setupResult.commands[0].command).toBe('echo "Hello Docker"')
    expect(setupResult.commands[0].success).toBe(true)
    expect(setupResult.commands[0].output).toContain('Hello Docker')

    // Cleanup
    await app.inject({
      method: 'DELETE',
      url: `/api/v1/repositories/${repository.id}`,
    })
  }, 30000)
})
