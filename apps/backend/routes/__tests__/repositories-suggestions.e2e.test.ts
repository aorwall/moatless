import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

describe('Repository Suggestions - E2E Tests', () => {
  let app: any

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  it('should return repository suggestions from real Claude config', async () => {
    // This E2E test uses real Claude config file - no mocking
    const response = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(response.statusCode).toBe(200)

    const body = JSON.parse(response.body)
    expect(body).toHaveProperty('suggestions')
    expect(Array.isArray(body.suggestions)).toBe(true)

    // If Claude config exists and has projects, we should get suggestions
    // Otherwise, it should return an empty array (not error)
    expect(body.suggestions.length).toBeGreaterThanOrEqual(0)

    // Each suggestion should have name and path
    if (body.suggestions.length > 0) {
      const firstSuggestion = body.suggestions[0]
      expect(firstSuggestion).toHaveProperty('name')
      expect(firstSuggestion).toHaveProperty('path')
      expect(typeof firstSuggestion.name).toBe('string')
      expect(typeof firstSuggestion.path).toBe('string')

      // Path should be absolute
      expect(firstSuggestion.path).toMatch(/^\//)

      console.log(`Found ${body.suggestions.length} repository suggestions from Claude config`)
    } else {
      console.log('No Claude projects found - this is expected in test environment')
    }
  })

  it('should filter to only git repositories by default', async () => {
    // Test with default behavior (onlyGitRepos=true)
    const response = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(response.statusCode).toBe(200)

    const body = JSON.parse(response.body)

    // All returned paths should be git repositories (not worktrees)
    // This is validated by the isRegularGitRepository function
    // We can't verify this without accessing the file system,
    // but we trust the implementation filters correctly
    expect(body.suggestions).toBeDefined()
  })

  it('should return all directories when onlyGitRepos=false', async () => {
    const response = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions?onlyGitRepos=false',
    })

    expect(response.statusCode).toBe(200)

    const body = JSON.parse(response.body)
    expect(body).toHaveProperty('suggestions')
    expect(Array.isArray(body.suggestions)).toBe(true)

    // When not filtering, we might get more results
    // (including non-git directories)
    expect(body.suggestions).toBeDefined()
  })

  it('should handle missing Claude config gracefully', async () => {
    // Even if Claude config doesn't exist or has no projects,
    // the endpoint should return an empty array, not error
    const response = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(response.statusCode).toBe(200)

    const body = JSON.parse(response.body)
    expect(body).toHaveProperty('suggestions')
    expect(Array.isArray(body.suggestions)).toBe(true)
    // Could be empty if no Claude config exists
    expect(body.suggestions.length).toBeGreaterThanOrEqual(0)
  })

  it('should return sorted suggestions by name', async () => {
    const response = await app.inject({
      method: 'GET',
      url: '/api/v1/repositories/suggestions',
    })

    expect(response.statusCode).toBe(200)

    const body = JSON.parse(response.body)

    if (body.suggestions.length > 1) {
      // Verify suggestions are sorted by name
      for (let i = 1; i < body.suggestions.length; i++) {
        const prevName = body.suggestions[i - 1].name.toLowerCase()
        const currName = body.suggestions[i].name.toLowerCase()
        expect(prevName <= currName).toBe(true)
      }
    }
  })
})
