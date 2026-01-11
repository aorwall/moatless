/**
 * Common test utilities for integration tests
 * Provides helpers for testing with both local and Docker environments
 */

import { promises as fs } from 'node:fs'
import type { CreateRepositoryRequest, RepositoryConfig } from '@moatless/types'
import type { TestApp } from '../../test/helper'
import { logger } from '../../utils'

export interface TestEnvironmentConfig {
  workspaceType: 'local' | 'docker'
  dockerImage?: string
  setupCommands?: string[]
  envVars?: Record<string, string>
}

/**
 * Creates a repository configuration for testing with the specified environment
 */
export function createTestRepositoryConfig(
  _tempDir: string,
  config: TestEnvironmentConfig,
): CreateRepositoryRequest {
  return {
    kind: 'remote',
    name: `integration-test-repository-${config.workspaceType}`,
    description: `Test repository for ${config.workspaceType} environment validation`,
    remoteUrl: 'https://github.com/test-owner/test-repo',
    provider: 'github',
    codeExecution: {
      setupCommands: config.setupCommands || ['echo "Repository setup complete"'],
      envVars: config.envVars,
      dockerImage: config.dockerImage || 'moatless-vibe-universal:latest',
    },
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
  logger.info(
    {
      repositoryId: repository.id,
      workspaceType: finalConfig.workspaceType,
    },
    'Test repository created',
  )

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
