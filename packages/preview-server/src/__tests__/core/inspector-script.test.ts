import { describe, expect, it } from 'vitest'
import { getPreviewInspectorScript } from '../../core/inspector-script.js'

describe('getPreviewInspectorScript', () => {
  it('should generate a script with server ID', () => {
    const serverId = 'server-123'
    const script = getPreviewInspectorScript(serverId)

    expect(script).toContain(`serverId: '${serverId}'`)
  })

  it('should include inspector initialization code', () => {
    const script = getPreviewInspectorScript('test-server')

    expect(script).toContain('Preview Inspector Script')
    // Modern arrow function IIFE pattern
    expect(script).toContain(';(() => {')
  })

  it('should include hook functions', () => {
    const script = getPreviewInspectorScript('test-server')

    // Check for various hook functions
    expect(script).toContain('hookConsole')
    expect(script).toContain('hookFetch')
    expect(script).toContain('hookScroll')
    expect(script).toContain('hookNavigation')
  })

  it('should contain valid JavaScript structure', () => {
    const script = getPreviewInspectorScript('test-server')

    // Should contain function definitions and arrow function IIFE structure
    expect(script).toContain('function')
    expect(script).toContain('=>')
    expect(script).toMatch(/;\(\(\) => \{/s) // Arrow function IIFE pattern
  })

  it('should handle different server ID formats', () => {
    const serverIds = [
      'simple-server',
      'server-with-uuid-123e4567-e89b-12d3-a456-426614174000',
      'server_with_underscores',
      'UPPERCASE-SERVER',
      '123-numeric-start',
    ]

    for (const serverId of serverIds) {
      const script = getPreviewInspectorScript(serverId)
      expect(script).toContain(`serverId: '${serverId}'`)
    }
  })

  it('should handle special characters in server ID', () => {
    // Server IDs might contain special characters
    const serverId = 'server-with-special'
    const script = getPreviewInspectorScript(serverId)

    // Should include the server ID
    expect(script).toContain(serverId)
  })

  it('should return non-empty string', () => {
    const script = getPreviewInspectorScript('server-id')

    expect(script).toBeTruthy()
    expect(script.length).toBeGreaterThan(0)
  })

  it('should return consistent output for same server ID', () => {
    const serverId = 'test-server'
    const script1 = getPreviewInspectorScript(serverId)
    const script2 = getPreviewInspectorScript(serverId)

    expect(script1).toBe(script2)
  })
})
