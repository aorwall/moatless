import type { FastifyInstance } from 'fastify'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  CreateEnvironmentRequest,
  EnvironmentResponse,
} from '../../../shared/types/environment.js'
import { build } from '../../test/helper.js'

// Mock the repository validation to avoid filesystem dependencies
vi.mock('../../utils/repository', () => ({
  validateRepositoryAccess: vi.fn().mockResolvedValue(undefined),
}))

describe('Environment Routes E2E', () => {
  let app: FastifyInstance
  let createdEnvironmentId: string

  beforeEach(async () => {
    app = await build()
    // Clear any existing test data
    const environments = await app.services.environment.listEnvironments()
    for (const env of environments) {
      if (env.name.includes('E2E Test')) {
        await app.services.environment.deleteEnvironment(env.id)
      }
    }
  })

  afterEach(async () => {
    // Cleanup created environment if test failed
    if (createdEnvironmentId) {
      try {
        await app.services.environment.deleteEnvironment(createdEnvironmentId)
      } catch (e) {
        // Environment might already be deleted
      }
    }
    await app.close()
  })

  describe('Complete CRUD Flow', () => {
    it('should handle complete environment lifecycle with local repository', async () => {
      // 1. Create environment
      const createRequest: CreateEnvironmentRequest = {
        name: 'E2E Test Local Environment',
        description: 'Test environment for E2E testing',
        repository: {
          type: 'local',
          path: process.cwd(), // Use current working directory which should be a git repo
          useGitWorktrees: false,
        },
        codeExecution: {
          type: 'local',
          setupCommands: 'echo "Setting up test environment"',
        },
        variables: {
          TEST_VAR: 'test-value',
          NODE_ENV: 'test',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: createRequest,
      })

      expect(createResponse.statusCode).toBe(200)
      const createdEnv: EnvironmentResponse = JSON.parse(createResponse.body)
      createdEnvironmentId = createdEnv.id

      // Validate created environment
      expect(createdEnv).toMatchObject({
        name: createRequest.name,
        description: createRequest.description,
        repository: createRequest.repository,
        codeExecution: createRequest.codeExecution,
        variables: createRequest.variables,
      })
      expect(createdEnv.id).toBeDefined()
      expect(createdEnv.createdAt).toBeDefined()
      expect(createdEnv.updatedAt).toBeDefined()

      // 2. Get the created environment
      const getResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })

      expect(getResponse.statusCode).toBe(200)
      const retrievedEnv: EnvironmentResponse = JSON.parse(getResponse.body)
      expect(retrievedEnv).toEqual(createdEnv)

      // 3. List environments and verify it's included
      const listResponse = await app.inject({
        method: 'GET',
        url: '/api/v1/environments',
      })

      expect(listResponse.statusCode).toBe(200)
      const { environments } = JSON.parse(listResponse.body)
      expect(environments).toContainEqual(expect.objectContaining({ id: createdEnvironmentId }))

      // 4. Update the environment
      const updateRequest = {
        name: 'Updated E2E Test Environment',
        description: 'Updated description for E2E test',
      }

      const updateResponse = await app.inject({
        method: 'PUT',
        url: `/api/v1/environments/${createdEnvironmentId}`,
        payload: updateRequest,
      })

      expect(updateResponse.statusCode).toBe(200)
      const updatedEnv: EnvironmentResponse = JSON.parse(updateResponse.body)
      expect(updatedEnv.name).toBe(updateRequest.name)
      expect(updatedEnv.description).toBe(updateRequest.description)
      // Repository and codeExecution should remain unchanged
      expect(updatedEnv.repository).toEqual(createdEnv.repository)
      expect(updatedEnv.codeExecution).toEqual(createdEnv.codeExecution)

      // 5. Delete the environment
      const deleteResponse = await app.inject({
        method: 'DELETE',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })

      expect(deleteResponse.statusCode).toBe(200)
      expect(JSON.parse(deleteResponse.body)).toEqual({ status: 'deleted' })

      // 6. Verify environment is deleted
      const getDeletedResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })

      expect(getDeletedResponse.statusCode).toBe(404)
      const errorBody = JSON.parse(getDeletedResponse.body)
      expect(errorBody.error.code).toBe('ENVIRONMENT_NOT_FOUND')

      // Clear the ID since it's been deleted
      createdEnvironmentId = ''
    })

    it('should handle complete environment lifecycle with GitHub repository', async () => {
      // 1. Create environment with GitHub repository
      const createRequest: CreateEnvironmentRequest = {
        name: 'E2E Test GitHub Environment',
        description: 'GitHub-based test environment',
        repository: {
          type: 'github',
          url: 'https://github.com/test-org/test-repo',
          repoId: 'test-org/test-repo',
          branch: 'main',
          token: 'github_pat_test_token',
        },
        codeExecution: {
          type: 'docker',
          image: 'node:18-alpine',
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
        url: '/api/v1/environments',
        payload: createRequest,
      })

      expect(createResponse.statusCode).toBe(200)
      const createdEnv: EnvironmentResponse = JSON.parse(createResponse.body)
      createdEnvironmentId = createdEnv.id

      // Validate GitHub-specific fields
      expect(createdEnv.repository.type).toBe('github')
      expect(createdEnv.codeExecution.type).toBe('docker')

      // Test file search endpoint
      const searchResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/environments/${createdEnvironmentId}/search-files`,
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
        url: `/api/v1/environments/${createdEnvironmentId}/branches`,
      })

      expect(branchesResponse.statusCode).toBe(200)
      const branchesResult = JSON.parse(branchesResponse.body)
      expect(branchesResult).toHaveProperty('branches')
      expect(branchesResult.branches).toBeInstanceOf(Array)

      // Cleanup
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })
      createdEnvironmentId = ''
    })
  })

  describe('Validation Tests', () => {
    it('should reject invalid create requests', async () => {
      // Missing required fields
      const invalidRequest1 = {
        name: 'Invalid Environment',
        // Missing repository and codeExecution
      }

      const response1 = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
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
        url: '/api/v1/environments',
        payload: invalidRequest2,
      })

      expect(response2.statusCode).toBe(400)

      // GitHub repository missing required fields
      const invalidRequest3 = {
        name: 'Invalid GitHub Repository',
        repository: {
          type: 'github',
          // Missing repoId and branch
        },
        codeExecution: {
          type: 'docker',
          image: 'node:18',
        },
      }

      const response3 = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: invalidRequest3,
      })

      expect(response3.statusCode).toBe(400)
    })

    it('should validate file search parameters', async () => {
      // Create a test environment first
      const createRequest: CreateEnvironmentRequest = {
        name: 'E2E Test Search Validation',
        repository: {
          type: 'local',
          path: process.cwd(),
        },
        codeExecution: {
          type: 'local',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: createRequest,
      })

      const env: EnvironmentResponse = JSON.parse(createResponse.body)
      createdEnvironmentId = env.id

      // Test with empty query
      const invalidSearchResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/environments/${env.id}/search-files`,
        payload: {
          query: '', // Empty query should fail validation
        },
      })

      expect(invalidSearchResponse.statusCode).toBe(400)

      // Test with maxResults exceeding limit
      const invalidMaxResultsResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/environments/${env.id}/search-files`,
        payload: {
          query: 'test',
          maxResults: 2000, // Exceeds max of 1000
        },
      })

      expect(invalidMaxResultsResponse.statusCode).toBe(400)

      // Cleanup
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })
      createdEnvironmentId = ''
    })
  })

  describe('Error Handling', () => {
    it('should handle not found errors correctly', async () => {
      const nonExistentId = 'non-existent-env-id'

      // GET non-existent environment
      const getResponse = await app.inject({
        method: 'GET',
        url: `/api/v1/environments/${nonExistentId}`,
      })

      expect(getResponse.statusCode).toBe(404)
      expect(JSON.parse(getResponse.body).error.code).toBe('ENVIRONMENT_NOT_FOUND')

      // UPDATE non-existent environment
      const updateResponse = await app.inject({
        method: 'PUT',
        url: `/api/v1/environments/${nonExistentId}`,
        payload: {
          name: 'Updated Name',
        },
      })

      expect(updateResponse.statusCode).toBe(404)

      // DELETE non-existent environment
      const deleteResponse = await app.inject({
        method: 'DELETE',
        url: `/api/v1/environments/${nonExistentId}`,
      })

      // Delete might return 200 or 404 depending on implementation
      expect([200, 404]).toContain(deleteResponse.statusCode)
    })

    it('should handle setup script endpoint', async () => {
      // Create environment first
      const createRequest: CreateEnvironmentRequest = {
        name: 'E2E Test Setup Script',
        repository: {
          type: 'local',
          path: process.cwd(),
        },
        codeExecution: {
          type: 'local',
          setupCommands: 'echo "Test setup"',
        },
      }

      const createResponse = await app.inject({
        method: 'POST',
        url: '/api/v1/environments',
        payload: createRequest,
      })

      const env: EnvironmentResponse = JSON.parse(createResponse.body)
      createdEnvironmentId = env.id

      // Run setup script
      const setupResponse = await app.inject({
        method: 'POST',
        url: `/api/v1/environments/${env.id}/run-setup`,
      })

      expect(setupResponse.statusCode).toBe(200)
      const setupResult = JSON.parse(setupResponse.body)
      expect(setupResult).toHaveProperty('success')
      expect(setupResult).toHaveProperty('output')

      // Cleanup
      await app.inject({
        method: 'DELETE',
        url: `/api/v1/environments/${createdEnvironmentId}`,
      })
      createdEnvironmentId = ''
    })
  })
})
