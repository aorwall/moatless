import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { CreateRepositoryRequest, RepositoryConfig } from '@moatless/shared/types/repository'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

// Mock the repository validation to avoid filesystem dependencies
vi.mock('../../utils/repository', () => ({
  validateRepositoryAccess: vi.fn().mockResolvedValue(undefined),
}))

describe('Repository Routes E2E', () => {
  let app: any
  let createdRepositoryId: string
  let tempDir: string

  beforeEach(async () => {
    // Create a temporary directory for our test repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-test-'))

    app = await build()

    // Clean up any existing repositories
    const repositories = await app.services.repository.listRepositories()
    for (const repo of repositories) {
      try {
        await app.services.repository.deleteRepository(repo.id)
      } catch (_e) {
        // Repository might already be deleted
      }
    }

    // Make sure temp directory exists and is a git repository
    await fs.mkdir(tempDir, { recursive: true })

    // Initialize git if not already initialized
    try {
      const { execSync } = require('node:child_process')
      execSync('git init', { cwd: tempDir, stdio: 'ignore' })
      execSync('git config user.name "Test User"', { cwd: tempDir, stdio: 'ignore' })
      execSync('git config user.email "test@example.com"', { cwd: tempDir, stdio: 'ignore' })

      // Create an initial commit
      const testFile = path.join(tempDir, 'test.txt')
      await fs.writeFile(testFile, 'Hello World')
      execSync('git add .', { cwd: tempDir, stdio: 'ignore' })
      execSync('git commit -m "Initial commit"', { cwd: tempDir, stdio: 'ignore' })
    } catch (_e) {
      // Git might already be initialized or other issues, continue anyway
    }

    // Clear any existing test data
    const repositories2 = await app.services.repository.listRepositories()
    for (const repo of repositories2) {
      if (repo.name.includes('E2E Test')) {
        // First delete any tasks associated with this repository
        const tasks = await app.services.task.listTasks()
        for (const task of tasks) {
          if (task.repositoryId === repo.id) {
            await app.services.task.deleteTask(task.taskId)
          }
        }
        // Now delete the repository
        await app.services.repository.deleteRepository(repo.id)
      }
    }
  })

  afterEach(async () => {
    // Clean up created repositories
    const repositories = await app.services.repository.listRepositories()
    for (const repo of repositories) {
      try {
        await app.services.repository.deleteRepository(repo.id)
      } catch (_e) {
        // Repository might already be deleted
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

    // Cleanup created repository if test failed
    if (createdRepositoryId) {
      try {
        await app.services.repository.deleteRepository(createdRepositoryId)
      } catch (_e) {
        // Repository might already be deleted
      }
    }
    await app.close()
  })

  describe('Complete CRUD Flow', () => {
    it('should handle complete repository lifecycle with local repository', async () => {
      // 1. Create repository
      const createRequest: CreateRepositoryRequest = {
        name: 'E2E Test Local Repository',
        description: 'Test repository for E2E testing',
        path: tempDir,
        useGitWorktrees: false,
        codeExecution: {
          type: 'local',
          setupCommands: 'echo "Setting up test repository"',
        },
        variables: {
          TEST_VAR: 'test-value',
          NODE_ENV: 'test',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: createRequest,
      })

      expect(createResponse.statusCode).toBe(200)
      const createdRepo: RepositoryConfig = JSON.parse(createResponse.body)
      createdRepositoryId = createdRepo.id

      // Validate created repository
      expect(createdRepo).toMatchObject({
        name: createRequest.name,
        description: createRequest.description,
        path: createRequest.path,
        useGitWorktrees: createRequest.useGitWorktrees,
        codeExecution: createRequest.codeExecution,
        variables: createRequest.variables,
      })
      expect(createdRepo.id).toBeDefined()
      expect(createdRepo.createdAt).toBeDefined()
      expect(createdRepo.updatedAt).toBeDefined()

      // 2. Get the created repository
      const getResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/repositories/${createdRepositoryId}`,
      })

      expect(getResponse.statusCode).toBe(200)
      const retrievedRepo: RepositoryConfig = JSON.parse(getResponse.body)
      expect(retrievedRepo).toEqual(createdRepo)

      // 3. List repositories and verify it's included
      const listResponse = await app.inject({
        method: 'GET',
        url: '/api/v1/repositories',
      })

      expect(listResponse.statusCode).toBe(200)
      const { repositories } = JSON.parse(listResponse.body)
      expect(repositories).toContainEqual(expect.objectContaining({ id: createdRepositoryId }))

      // 4. Update the repository
      const updateRequest = {
        name: 'Updated E2E Test Repository',
        description: 'Updated description for E2E test',
      }

      const updateResponse = await app.inject({
        method: 'PUT',
        url: `/api/v1/repositories/${createdRepositoryId}`,
        payload: updateRequest,
      })

      expect(updateResponse.statusCode).toBe(200)
      const updatedRepo: RepositoryConfig = JSON.parse(updateResponse.body)
      expect(updatedRepo.name).toBe(updateRequest.name)
      expect(updatedRepo.description).toBe(updateRequest.description)
      // Path, useGitWorktrees and codeExecution should remain unchanged
      expect(updatedRepo.path).toEqual(createdRepo.path)
      expect(updatedRepo.useGitWorktrees).toEqual(createdRepo.useGitWorktrees)
      expect(updatedRepo.codeExecution).toEqual(createdRepo.codeExecution)

      // 5. Delete the repository
      const deleteResponse = await app.inject({
        method: 'DELETE',
        url: `/api/v1/repositories/${createdRepositoryId}`,
      })

      expect(deleteResponse.statusCode).toBe(200)
      expect(JSON.parse(deleteResponse.body)).toEqual({ status: 'deleted' })

      // 6. Verify repository is deleted
      const getDeletedResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/repositories/${createdRepositoryId}`,
      })

      expect(getDeletedResponse.statusCode).toBe(404)
      const errorBody = JSON.parse(getDeletedResponse.body)
      expect(errorBody.error.code).toBe('REPOSITORY_NOT_FOUND')

      // Clear the ID since it's been deleted
      createdRepositoryId = ''
    })

    it('should handle complete repository lifecycle with Docker execution', async () => {
      // 1. Create repository with Docker execution
      const createRequest: CreateRepositoryRequest = {
        name: 'E2E Test Docker Repository',
        description: 'Docker-based test repository',
        path: tempDir,
        useGitWorktrees: false,
        codeExecution: {
          type: 'docker',
          dockerImage: 'moatless-vibe-universal:latest',
          setupCommands: 'npm ci\nnpm run build',
          envVars: {
            NODE_ENV: 'production',
            PORT: '3000',
          },
          ports: {
            '3000': 3000,
            '8080': 8080,
          },
        },
        variables: {
          API_KEY: 'test-api-key',
          DATABASE_URL: 'postgresql://localhost:5432/testdb',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: createRequest,
      })

      expect(createResponse.statusCode).toBe(200)
      const createdRepo: RepositoryConfig = JSON.parse(createResponse.body)
      createdRepositoryId = createdRepo.id

      // Validate Docker-specific fields
      expect(createdRepo.codeExecution.type).toBe('docker')

      // Test file search endpoint
      const searchResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/repositories/${createdRepositoryId}/search-files`,
        payload: {
          query: 'test',
          filePattern: '*.ts',
          maxResults: 10,
        },
      })

      expect(searchResponse.statusCode).toBe(200)
      const searchResult = JSON.parse(searchResponse.body)
      expect(searchResult).toHaveProperty('files')
      expect(searchResult).toHaveProperty('totalCount')
      expect(searchResult).toHaveProperty('truncated')

      // Test branches endpoint
      const branchesResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/repositories/${createdRepositoryId}/branches`,
      })

      expect(branchesResponse.statusCode).toBe(200)
      const branchesResult = JSON.parse(branchesResponse.body)
      expect(branchesResult).toHaveProperty('branches')
      expect(branchesResult.branches).toBeInstanceOf(Array)

      // Cleanup
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/repositories/${createdRepositoryId}`,
      })
      createdRepositoryId = ''
    })
  })

  describe('Validation Tests', () => {
    it('should reject invalid create requests', async () => {
      // Missing required fields
      const invalidRequest1 = {
        name: 'Invalid Repository',
        // Missing repository and codeExecution
      }

      const response1 = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: invalidRequest1,
      })

      expect(response1.statusCode).toBe(400)

      // Invalid repository type
      const invalidRequest2 = {
        name: 'Invalid Repository Type',
        repository: {
          type: 'invalid-type',
          path: '/tmp/test',
        },
        codeExecution: {
          type: 'local',
        },
      }

      const response2 = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: invalidRequest2,
      })

      expect(response2.statusCode).toBe(400)

      // Docker repository missing required fields
      const invalidRequest3 = {
        name: 'Invalid Docker Repository',
        repository: {
          type: 'invalid' as any,
          // Invalid type
        },
        codeExecution: {
          type: 'docker',
          dockerImage: 'moatless-vibe-universal:latest',
        },
      }

      const response3 = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: invalidRequest3,
      })

      expect(response3.statusCode).toBe(400)
    })

    it('should validate file search parameters', async () => {
      // Create a test repository first
      const createRequest: CreateRepositoryRequest = {
        name: 'E2E Test Search Validation',
        path: tempDir,
        codeExecution: {
          type: 'local',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/repositories',
        payload: createRequest,
      })

      const repo: RepositoryConfig = JSON.parse(createResponse.body)
      createdRepositoryId = repo.id

      // Test with empty query
      const invalidSearchResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/repositories/${repo.id}/search-files`,
        payload: {
          query: '', // Empty query should fail validation
        },
      })

      expect(invalidSearchResponse.statusCode).toBe(400)

      // Test with maxResults exceeding limit
      const invalidMaxResultsResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/repositories/${repo.id}/search-files`,
        payload: {
          query: 'test',
          maxResults: 2000, // Exceeds max of 1000
        },
      })

      expect(invalidMaxResultsResponse.statusCode).toBe(400)

      // Cleanup
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/repositories/${createdRepositoryId}`,
      })
      createdRepositoryId = ''
    })
  })

  describe('Error Handling', () => {
    it('should handle not found errors correctly', async () => {
      const nonExistentId = 'non-existent-repo-id'

      // GET non-existent repository
      const getResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/repositories/${nonExistentId}`,
      })

      expect(getResponse.statusCode).toBe(404)
      expect(JSON.parse(getResponse.body).error.code).toBe('REPOSITORY_NOT_FOUND')

      // UPDATE non-existent repository
      const updateResponse = await app.inject({
        method: 'PUT',
        url: `/api/v1/repositories/${nonExistentId}`,
        payload: {
          name: 'Updated Name',
        },
      })

      expect(updateResponse.statusCode).toBe(404)

      // DELETE non-existent repository
      const deleteResponse = await app.inject({
        method: 'DELETE',
        url: `/api/v1/repositories/${nonExistentId}`,
      })

      // Delete might return 200 or 404 depending on implementation
      expect([200, 404]).toContain(deleteResponse.statusCode)
    })
  })
})

describe('Repositories E2E - Additional Tests', () => {
  let app: any
  let tempDir: string

  beforeEach(async () => {
    // Create a temporary directory for our test repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-test-'))

    app = await build()

    // Clean up any existing repositories
    const repositories = await app.services.repository.listRepositories()
    for (const repo of repositories) {
      try {
        await app.services.repository.deleteRepository(repo.id)
      } catch (_e) {
        // Repository might already be deleted
      }
    }

    // Make sure temp directory exists and is a git repository
    await fs.mkdir(tempDir, { recursive: true })

    // Initialize git if not already initialized
    try {
      const { execSync } = require('node:child_process')
      execSync('git init', { cwd: tempDir, stdio: 'ignore' })
      execSync('git config user.name "Test User"', { cwd: tempDir, stdio: 'ignore' })
      execSync('git config user.email "test@example.com"', { cwd: tempDir, stdio: 'ignore' })

      // Create an initial commit
      const testFile = path.join(tempDir, 'test.txt')
      await fs.writeFile(testFile, 'Hello World')
      execSync('git add .', { cwd: tempDir, stdio: 'ignore' })
      execSync('git commit -m "Initial commit"', { cwd: tempDir, stdio: 'ignore' })
    } catch (_e) {
      // Git might already be initialized or other issues, continue anyway
    }
  })

  afterEach(async () => {
    // Clean up created repositories
    const repositories = await app.services.repository.listRepositories()
    for (const repo of repositories) {
      try {
        await app.services.repository.deleteRepository(repo.id)
      } catch (_e) {
        // Repository might already be deleted
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

  it('should create, list, and delete repositories', async () => {
    // Create a repository
    const createRepositoryRequest: CreateRepositoryRequest = {
      name: 'Test Repository',
      description: 'A test repository for E2E testing',
      path: tempDir,
      useGitWorktrees: false,
      codeExecution: {
        type: 'local',
      },
    }

    // Test repository creation
    const response = await app.inject({
      method: 'POST',
      url: '/api/v1/repositories',
      payload: createRepositoryRequest,
    })

    expect(response.statusCode).toBe(200)
    const createdRepository: RepositoryConfig = JSON.parse(response.body)
    expect(createdRepository.id).toBeTruthy()
    expect(createdRepository.name).toBe('Test Repository')

    // Test repository listing
    const listResponse = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories',
    })

    expect(listResponse.statusCode).toBe(200)
    const { repositories } = JSON.parse(listResponse.body)
    expect(repositories).toHaveLength(1)
    expect(repositories[0].id).toBe(createdRepository.id)

    // Test repository deletion
    const deleteResponse = await app.inject({
      method: 'DELETE',
      url: `/api/v1/repositories/${createdRepository.id}`,
    })

    expect(deleteResponse.statusCode).toBe(200)

    // Verify repository is deleted
    const finalListResponse = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories',
    })

    expect(finalListResponse.statusCode).toBe(200)
    const { repositories: finalRepositories } = JSON.parse(finalListResponse.body)
    expect(finalRepositories).toHaveLength(0)
  })
})
