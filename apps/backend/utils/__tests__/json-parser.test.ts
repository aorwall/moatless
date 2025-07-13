import { describe, expect, it } from 'vitest'
import { parseBranchNameAndSummary, parseJsonFromText } from '../json-parser.js'

describe('parseJsonFromText', () => {
  it('should parse valid JSON from clean text', () => {
    const text = '{"branchName": "feature/login", "summary": "Add login functionality"}'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/login',
      summary: 'Add login functionality',
    })
    expect(result.error).toBeUndefined()
  })

  it('should parse JSON from text with markdown code blocks', () => {
    const text = '```json\n{"branchName": "feature/auth"}\n```'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(true)
    expect(result.data).toEqual({ branchName: 'feature/auth' })
  })

  it('should parse JSON from text with surrounding content', () => {
    const text = 'Here is the response: {"branchName": "fix/bug-123"} and that is it.'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(true)
    expect(result.data).toEqual({ branchName: 'fix/bug-123' })
  })

  it('should handle multiple JSON objects and return the first valid one', () => {
    const text = '{"invalid": "data"} {"branchName": "feature/test", "summary": "Test feature"}'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/test',
      summary: 'Test feature',
    })
  })

  it('should return false when no JSON found', () => {
    const text = 'This is just plain text with no JSON'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(false)
    expect(result.data).toBeNull()
    expect(result.error).toBe('No JSON object found with required keys: branchName')
  })

  it('should return false when JSON does not have required keys', () => {
    const text = '{"someOtherKey": "value"}'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(false)
    expect(result.data).toBeNull()
    expect(result.error).toBe('No JSON object found with required keys: branchName')
  })

  it('should handle invalid JSON gracefully', () => {
    const text = '{"branchName": invalid json}'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(false)
    expect(result.data).toBeNull()
  })

  it('should handle empty or null input', () => {
    expect(parseJsonFromText('', ['branchName'])).toEqual({
      data: null,
      found: false,
      error: 'Invalid input text',
    })

    expect(parseJsonFromText(null as any, ['branchName'])).toEqual({
      data: null,
      found: false,
      error: 'Invalid input text',
    })
  })

  it('should work without required keys', () => {
    const text = '{"any": "data"}'
    const result = parseJsonFromText(text)

    expect(result.found).toBe(true)
    expect(result.data).toEqual({ any: 'data' })
  })

  it('should handle nested JSON objects', () => {
    const text = '{"branchName": "feature/nested", "details": {"author": "user"}}'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/nested',
      details: { author: 'user' },
    })
  })

  it('should not match arrays as valid objects', () => {
    const text = '["branchName", "feature/array"]'
    const result = parseJsonFromText(text, ['branchName'])

    expect(result.found).toBe(false)
    expect(result.data).toBeNull()
  })
})

describe('parseBranchNameAndSummary', () => {
  it('should parse branch name and summary', () => {
    const text = '{"branchName": "feature/login", "summary": "Add user authentication"}'
    const result = parseBranchNameAndSummary(text)

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/login',
      summary: 'Add user authentication',
    })
  })

  it('should parse branch name without summary', () => {
    const text = '{"branchName": "fix/bug-456"}'
    const result = parseBranchNameAndSummary(text)

    expect(result.found).toBe(true)
    expect(result.data).toEqual({ branchName: 'fix/bug-456' })
  })

  it('should handle Claude Code response format', () => {
    const text = `Here's a suitable branch name for your request:

\`\`\`json
{"branchName": "feature/metrics-export", "summary": "Add usage metrics tracking and export functionality"}
\`\`\`

This branch name follows standard conventions and clearly describes the feature being implemented.`

    const result = parseBranchNameAndSummary(text)

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/metrics-export',
      summary: 'Add usage metrics tracking and export functionality',
    })
  })

  it('should handle new few-shot example format responses', () => {
    const text =
      '{"branchName": "feature/user-authentication", "summary": "Add user login and logout functionality"}'
    const result = parseBranchNameAndSummary(text)

    expect(result.found).toBe(true)
    expect(result.data).toEqual({
      branchName: 'feature/user-authentication',
      summary: 'Add user login and logout functionality',
    })
  })

  it('should return false when branchName is missing', () => {
    const text = '{"summary": "Some summary without branch name"}'
    const result = parseBranchNameAndSummary(text)

    expect(result.found).toBe(false)
    expect(result.data).toBeNull()
  })
})
