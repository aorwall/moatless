import { promises as fs } from 'fs'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentService } from '../agent.js'
import { EnvironmentService } from '../environment.js'
import { EventService } from '../event.js'
import { FileStorage } from '../storage.js'
import { TaskService } from '../task.js'
import { WorkspaceService } from '../workspace.js'

// Mock the Claude CLI
vi.mock('child_process', async () => {
  const actual = await vi.importActual('child_process')
  return {
    ...actual,
    spawn: vi.fn(() => {
      // Create a mock child process
      const mockProcess = {
        stdout: {
          on: vi.fn(),
          pipe: vi.fn(),
        },
        stderr: {
          on: vi.fn(),
        },
        on: vi.fn((event, handler) => {
          if (event === 'close') {
            // Simulate process closing successfully
            setTimeout(() => handler(0), 10)
          }
        }),
        kill: vi.fn(),
        killed: false,
      }
      return mockProcess
    }),
  }
})

describe('AgentService', () => {
  let service: AgentService
  let storage: FileStorage
  let eventService: EventService
  let taskService: TaskService
  let workspaceService: WorkspaceService
  let environmentService: EnvironmentService
  let tempDir: string

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now())
    await fs.mkdir(tempDir, { recursive: true })

    // Initialize as a git repository
    const { exec } = await import('child_process')
    const { promisify } = await import('util')
    const execAsync = promisify(exec)
    await execAsync('git init', { cwd: tempDir })
    await execAsync('git config user.email "test@example.com"', { cwd: tempDir })
    await execAsync('git config user.name "Test User"', { cwd: tempDir })
    await fs.writeFile(path.join(tempDir, 'README.md'), '# Test Repository')
    await execAsync('git add .', { cwd: tempDir })
    await execAsync('git commit -m "Initial commit"', { cwd: tempDir })

    storage = new FileStorage(tempDir)
    eventService = new EventService(storage)
    environmentService = new EnvironmentService(storage)
    workspaceService = new WorkspaceService(storage, environmentService, eventService)
    taskService = new TaskService(storage, eventService, environmentService, workspaceService)
    service = new AgentService(taskService, workspaceService, eventService)
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('runAgent', () => {
    it('should start agent execution', async () => {
      // Create test environment and workspace
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false,
      })

      await workspaceService.createWorkspace(taskId, environment.id)

      const executionId = await service.runAgent(taskId, 'Hello world')

      expect(executionId).toBeDefined()

      const execution = await service.getAgentExecution(executionId)
      expect(execution).toBeDefined()
      expect(execution?.taskId).toBe(taskId)
      expect(execution?.status).toBe('running')
    })

    it('should prevent multiple agents for same task', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false,
      })

      await workspaceService.createWorkspace(taskId, environment.id)

      // Start first agent
      await service.runAgent(taskId, 'First message')

      // Try to start second agent - should fail
      await expect(service.runAgent(taskId, 'Second message')).rejects.toThrow(
        'Agent is already running',
      )
    })
  })

  describe('stopAgent', () => {
    it('should stop running agent', async () => {
      const environment = await environmentService.createEnvironment({
        name: 'Test Environment',
        repository: {
          type: 'local',
          path: tempDir,
        },
        codeExecution: {
          type: 'local',
        },
      })

      const taskId = await taskService.createTask({
        agentType: 'claude-code',
        environmentId: environment.id,
        message: 'Test task',
        start: false,
      })

      await workspaceService.createWorkspace(taskId, environment.id)

      const executionId = await service.runAgent(taskId, 'Hello world')

      // Stop the agent
      await service.stopAgent(executionId)

      const execution = await service.getAgentExecution(executionId)
      expect(execution?.status).toBe('stopped')
      expect(execution?.completedAt).toBeDefined()
    })
  })
})
