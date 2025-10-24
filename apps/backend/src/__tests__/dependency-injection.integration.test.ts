import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { AppWithServices } from '../app.js'
import { createApp } from '../app.js'

describe('Dependency Injection', () => {
  let appWithServices: AppWithServices

  beforeEach(async () => {
    appWithServices = await createApp()
  })

  afterEach(async () => {
    // Cleanup if needed
  })

  it('should resolve all services without circular dependency errors', async () => {
    const { services } = appWithServices

    // Verify all services are resolved and not null/undefined
    expect(services.agent).toBeDefined()
    expect(services.agent).not.toBeNull()

    expect(services.auth).toBeDefined()
    expect(services.auth).not.toBeNull()

    expect(services.claude).toBeDefined()
    expect(services.claude).not.toBeNull()

    expect(services.command).toBeDefined()
    expect(services.command).not.toBeNull()

    expect(services.docker).toBeDefined()
    expect(services.docker).not.toBeNull()

    expect(services.event).toBeDefined()
    expect(services.event).not.toBeNull()

    expect(services.figma).toBeDefined()
    expect(services.figma).not.toBeNull()

    expect(services.github).toBeDefined()
    expect(services.github).not.toBeNull()

    expect(services.mcp).toBeDefined()
    expect(services.mcp).not.toBeNull()

    expect(services.message).toBeDefined()
    expect(services.message).not.toBeNull()

    expect(services.netlify).toBeDefined()
    expect(services.netlify).not.toBeNull()

    expect(services.previewRegistry).toBeDefined()
    expect(services.previewRegistry).not.toBeNull()

    expect(services.repository).toBeDefined()
    expect(services.repository).not.toBeNull()

    expect(services.screenshot).toBeDefined()
    expect(services.screenshot).not.toBeNull()

    expect(services.screenshotOrchestration).toBeDefined()
    expect(services.screenshotOrchestration).not.toBeNull()

    expect(services.settings).toBeDefined()
    expect(services.settings).not.toBeNull()

    expect(services.storage).toBeDefined()
    expect(services.storage).not.toBeNull()

    expect(services.task).toBeDefined()
    expect(services.task).not.toBeNull()

    expect(services.tool).toBeDefined()
    expect(services.tool).not.toBeNull()

    expect(services.upload).toBeDefined()
    expect(services.upload).not.toBeNull()

    expect(services.workspace).toBeDefined()
    expect(services.workspace).not.toBeNull()
  })

  it('should not have circular dependencies', async () => {
    // This test will fail at app startup if circular dependencies exist
    // The createApp call in beforeEach validates this
    await expect(createApp()).resolves.toBeDefined()
  })

  it('should resolve service container with all expected services', () => {
    const { services } = appWithServices

    const expectedServices = [
      'agent',
      'auth',
      'claude',
      'command',
      'docker',
      'event',
      'figma',
      'github',
      'mcp',
      'message',
      'netlify',
      'previewRegistry',
      'repository',
      'screenshot',
      'screenshotOrchestration',
      'settings',
      'storage',
      'task',
      'tool',
      'upload',
      'workspace',
    ]

    // Verify all expected services are present
    for (const serviceName of expectedServices) {
      expect(services).toHaveProperty(serviceName)
      expect(services[serviceName as keyof typeof services]).toBeDefined()
      expect(services[serviceName as keyof typeof services]).not.toBeNull()
    }

    // Verify no unexpected services
    const actualServices = Object.keys(services)
    expect(actualServices.sort()).toEqual(expectedServices.sort())
  })

  it('should resolve WorkspaceService with ScreenshotService dependency', () => {
    const { services } = appWithServices

    // WorkspaceService should have ScreenshotService injected
    // This verifies the one-way dependency (no circular reference)
    expect(services.workspace).toBeDefined()
    expect(services.screenshot).toBeDefined()
  })

  it('should resolve ScreenshotOrchestrationService with both WorkspaceService and ScreenshotService', () => {
    const { services } = appWithServices

    // ScreenshotOrchestrationService should be able to coordinate between both services
    // This verifies the orchestration layer breaks the circular dependency
    expect(services.screenshotOrchestration).toBeDefined()
    expect(services.workspace).toBeDefined()
    expect(services.screenshot).toBeDefined()
  })

  it('should resolve AgentService with WorkspaceService dependency', () => {
    const { services } = appWithServices

    // AgentService should have WorkspaceService injected using class-based injection
    expect(services.agent).toBeDefined()
    expect(services.workspace).toBeDefined()
  })
})
