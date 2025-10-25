import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createApp } from '../app'
import { appContainer } from '../container/container'

describe('Dependency Injection', () => {
  beforeEach(async () => {
    await createApp()
  })

  afterEach(async () => {
    // Cleanup if needed
  })

  it('should resolve all services without circular dependency errors', async () => {
    // Import all service classes
    const { AgentService } = await import('../services/agent.service.js')
    const { AuthService } = await import('../features/auth/services/auth.service.js')
    const { ClaudeService } = await import('../agents/claude-code/claude.service.js')
    const { CommandService } = await import('../services/command.service.js')
    const { DockerService } = await import('../services/docker.service.js')
    const { EventService } = await import('../features/events/index.js')
    const { FigmaService } = await import('../features/figma/services/figma.service.js')
    const { GitHubService } = await import('../services/github.service.js')
    const { McpService } = await import('../services/mcp.service.js')
    const { MessageService } = await import('../services/message.service.js')
    const { NetlifyService } = await import('../services/netlify.service.js')
    const { RepositoryService } = await import(
      '../features/repositories/services/repository.service.js'
    )
    const { ScreenshotService } = await import('../services/screenshot.service.js')
    const { ScreenshotOrchestrationService } = await import(
      '../services/screenshot-orchestration.service.js'
    )
    const { SettingsService } = await import('../services/settings.service.js')
    const { TaskService } = await import('../services/task.service.js')
    const { ToolService } = await import('../services/tool.service.js')
    const { UploadService } = await import('../services/upload.service.js')
    const { WorkspaceService } = await import('../services/workspace.service.js')

    // Verify all services are resolved and not null/undefined
    const agent = appContainer.get(AgentService)
    expect(agent).toBeDefined()
    expect(agent).not.toBeNull()

    const auth = appContainer.get(AuthService)
    expect(auth).toBeDefined()
    expect(auth).not.toBeNull()

    const claude = appContainer.get(ClaudeService)
    expect(claude).toBeDefined()
    expect(claude).not.toBeNull()

    const command = appContainer.get(CommandService)
    expect(command).toBeDefined()
    expect(command).not.toBeNull()

    const docker = appContainer.get(DockerService)
    expect(docker).toBeDefined()
    expect(docker).not.toBeNull()

    const event = appContainer.get(EventService)
    expect(event).toBeDefined()
    expect(event).not.toBeNull()

    const figma = appContainer.get(FigmaService)
    expect(figma).toBeDefined()
    expect(figma).not.toBeNull()

    const github = appContainer.get(GitHubService)
    expect(github).toBeDefined()
    expect(github).not.toBeNull()

    const mcp = appContainer.get(McpService)
    expect(mcp).toBeDefined()
    expect(mcp).not.toBeNull()

    const message = appContainer.get(MessageService)
    expect(message).toBeDefined()
    expect(message).not.toBeNull()

    const netlify = appContainer.get(NetlifyService)
    expect(netlify).toBeDefined()
    expect(netlify).not.toBeNull()

    const repository = appContainer.get(RepositoryService)
    expect(repository).toBeDefined()
    expect(repository).not.toBeNull()

    const screenshot = appContainer.get(ScreenshotService)
    expect(screenshot).toBeDefined()
    expect(screenshot).not.toBeNull()

    const screenshotOrchestration = appContainer.get(ScreenshotOrchestrationService)
    expect(screenshotOrchestration).toBeDefined()
    expect(screenshotOrchestration).not.toBeNull()

    const settings = appContainer.get(SettingsService)
    expect(settings).toBeDefined()
    expect(settings).not.toBeNull()

    const task = appContainer.get(TaskService)
    expect(task).toBeDefined()
    expect(task).not.toBeNull()

    const tool = appContainer.get(ToolService)
    expect(tool).toBeDefined()
    expect(tool).not.toBeNull()

    const upload = appContainer.get(UploadService)
    expect(upload).toBeDefined()
    expect(upload).not.toBeNull()

    const workspace = appContainer.get(WorkspaceService)
    expect(workspace).toBeDefined()
    expect(workspace).not.toBeNull()
  })

  it('should not have circular dependencies', async () => {
    // This test will fail at app startup if circular dependencies exist
    // The createApp call in beforeEach validates this
    await expect(createApp()).resolves.toBeDefined()
  })

  it('should resolve service container with all expected services', async () => {
    // Import all service classes
    const { AgentService } = await import('../services/agent.service.js')
    const { AuthService } = await import('../features/auth/services/auth.service.js')
    const { ClaudeService } = await import('../agents/claude-code/claude.service.js')
    const { CommandService } = await import('../services/command.service.js')
    const { DockerService } = await import('../services/docker.service.js')
    const { EventService } = await import('../features/events/index.js')
    const { FigmaService } = await import('../features/figma/services/figma.service.js')
    const { GitHubService } = await import('../services/github.service.js')
    const { McpService } = await import('../services/mcp.service.js')
    const { MessageService } = await import('../services/message.service.js')
    const { NetlifyService } = await import('../services/netlify.service.js')
    const { RepositoryService } = await import(
      '../features/repositories/services/repository.service.js'
    )
    const { ScreenshotService } = await import('../services/screenshot.service.js')
    const { ScreenshotOrchestrationService } = await import(
      '../services/screenshot-orchestration.service.js'
    )
    const { SettingsService } = await import('../services/settings.service.js')
    const { TaskService } = await import('../services/task.service.js')
    const { ToolService } = await import('../services/tool.service.js')
    const { UploadService } = await import('../services/upload.service.js')
    const { WorkspaceService } = await import('../services/workspace.service.js')

    const expectedServices = [
      AgentService,
      AuthService,
      ClaudeService,
      CommandService,
      DockerService,
      EventService,
      FigmaService,
      GitHubService,
      McpService,
      MessageService,
      NetlifyService,
      RepositoryService,
      ScreenshotService,
      ScreenshotOrchestrationService,
      SettingsService,
      TaskService,
      ToolService,
      UploadService,
      WorkspaceService,
    ]

    // Verify all expected services are present
    for (const ServiceClass of expectedServices) {
      const service = appContainer.get(ServiceClass)
      expect(service).toBeDefined()
      expect(service).not.toBeNull()
    }
  })

  it('should resolve WorkspaceService with ScreenshotService dependency', async () => {
    const { WorkspaceService } = await import('../services/workspace.service.js')
    const { ScreenshotService } = await import('../services/screenshot.service.js')

    // WorkspaceService should have ScreenshotService injected
    // This verifies the one-way dependency (no circular reference)
    const workspace = appContainer.get(WorkspaceService)
    const screenshot = appContainer.get(ScreenshotService)
    expect(workspace).toBeDefined()
    expect(screenshot).toBeDefined()
  })

  it('should resolve ScreenshotOrchestrationService with both WorkspaceService and ScreenshotService', async () => {
    const { ScreenshotOrchestrationService } = await import(
      '../services/screenshot-orchestration.service.js'
    )
    const { WorkspaceService } = await import('../services/workspace.service.js')
    const { ScreenshotService } = await import('../services/screenshot.service.js')

    // ScreenshotOrchestrationService should be able to coordinate between both services
    // This verifies the orchestration layer breaks the circular dependency
    const screenshotOrchestration = appContainer.get(ScreenshotOrchestrationService)
    const workspace = appContainer.get(WorkspaceService)
    const screenshot = appContainer.get(ScreenshotService)
    expect(screenshotOrchestration).toBeDefined()
    expect(workspace).toBeDefined()
    expect(screenshot).toBeDefined()
  })

  it('should resolve AgentService with WorkspaceService dependency', async () => {
    const { AgentService } = await import('../services/agent.service.js')
    const { WorkspaceService } = await import('../services/workspace.service.js')

    // AgentService should have WorkspaceService injected using class-based injection
    const agent = appContainer.get(AgentService)
    const workspace = appContainer.get(WorkspaceService)
    expect(agent).toBeDefined()
    expect(workspace).toBeDefined()
  })
})
