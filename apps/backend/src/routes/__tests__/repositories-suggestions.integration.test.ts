import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../../test/helper.js'

describe('Repository Routes - Suggestions - Integration Tests', () => {
  let app: any

  beforeEach(async () => {
    // Mock the specific modules we need for route testing
    vi.mock('../../utils/git-utils.js', () => ({
      isRegularGitRepository: vi.fn(),
    }))

    app = await build()

    // Mock the ClaudeService methods after app is built
    vi.spyOn(app.services.claude, 'getClaudeProjectDirectories')

    // Mock fs.stat on the repository service instance
    vi.spyOn(app.services.repository, 'getSuggestedRepositories')
  })

  afterEach(async () => {
    if (app) {
      await app.close()
    }
    vi.clearAllMocks()
    vi.resetModules()
  })

  it('should return suggested repositories from Claude projects', async () => {
    // Mock the repository service method directly
    vi.mocked(app.services.repository.getSuggestedRepositories).mockResolvedValue([
      { name: 'repo1', path: '/path/to/repo1' },
      { name: 'repo2', path: '/path/to/repo2' },
    ])

    const res = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(res.statusCode).toBe(200)

    const data = JSON.parse(res.body)
    expect(data).toEqual({
      suggestions: [
        { name: 'repo1', path: '/path/to/repo1' },
        { name: 'repo2', path: '/path/to/repo2' },
      ],
    })
  })

  it('should pass query parameter to service method', async () => {
    // Mock the repository service method
    vi.mocked(app.services.repository.getSuggestedRepositories).mockResolvedValue([
      { name: 'dir1', path: '/path/to/dir1' },
    ])

    const res = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions?onlyGitRepos=false',
    })

    expect(res.statusCode).toBe(200)

    // Verify the service was called with the correct parameter
    expect(app.services.repository.getSuggestedRepositories).toHaveBeenCalledWith(false)
  })

  it('should handle service errors', async () => {
    // Mock service to throw an error
    vi.mocked(app.services.repository.getSuggestedRepositories).mockRejectedValue(
      new Error('Service error'),
    )

    const res = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    // When the service throws an error, the route will return 500
    expect(res.statusCode).toBe(500)
  })

  it('should default onlyGitRepos to true', async () => {
    vi.mocked(app.services.repository.getSuggestedRepositories).mockResolvedValue([])

    const res = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(res.statusCode).toBe(200)

    // Verify the service was called with true (default)
    expect(app.services.repository.getSuggestedRepositories).toHaveBeenCalledWith(true)
  })

  it('should handle onlyGitRepos=true explicitly', async () => {
    vi.mocked(app.services.repository.getSuggestedRepositories).mockResolvedValue([])

    const res = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions?onlyGitRepos=true',
    })

    expect(res.statusCode).toBe(200)

    // Verify the service was called with true
    expect(app.services.repository.getSuggestedRepositories).toHaveBeenCalledWith(true)
  })
})
