import { vi } from 'vitest'
import type { BaseAgent } from '../../agents/base-agent.js'
import { TestAgent } from '../../agents/test-agent.js'
import type { BaseEnvironment } from '../../environments/base-environment.js'
import type { Workspace } from '../../workspaces/workspace.js'

/**
 * Mock factory for creating consistent workspace-related mocks
 * Following the Strategic Mocking Pattern
 */

export function createMockAgent(overrides?: Partial<BaseAgent> & { options?: any }): BaseAgent {
  // Use TestAgent for consistent test behavior
  const agent = new TestAgent('test-task-mock', overrides?.options || {})

  // Apply any overrides (except options which was already used)
  if (overrides) {
    const { options, ...rest } = overrides
    Object.assign(agent, rest)
  }

  return agent
}

export function createMockEnvironment(overrides?: Partial<BaseEnvironment>): BaseEnvironment {
  return {
    createAgent: vi.fn().mockResolvedValue(createMockAgent()),
    stopAgent: vi.fn().mockResolvedValue(true),
    getType: vi.fn().mockReturnValue('local'),
    getStatus: vi.fn().mockReturnValue('ready'),
    getError: vi.fn().mockReturnValue(undefined),
    getCreatedAt: vi.fn().mockReturnValue(new Date()),
    getWorkspacePath: vi.fn().mockReturnValue('/mock/workspace/path'),
    runCommand: vi.fn().mockResolvedValue({
      command: 'echo test',
      output: 'test',
      exitCode: 0,
      durationMs: 100,
    }),
    isRunning: vi.fn().mockReturnValue(false),
    setup: vi.fn().mockResolvedValue(undefined),
    teardown: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as BaseEnvironment
}

export function createMockWorkspace(overrides?: Partial<Workspace>): Workspace {
  const mockEnvironment = createMockEnvironment()

  return {
    id: 'test-task-123',
    type: 'local',
    status: 'ready',
    repositoryId: 'test-repo-id',
    getStatus: vi.fn().mockReturnValue('idle'),
    isWorkspacePathValid: vi.fn().mockResolvedValue(true),
    getWorkspacePath: vi.fn().mockReturnValue('/mock/workspace/path'),
    setup: vi.fn().mockResolvedValue(undefined),
    getEnvironment: vi.fn().mockReturnValue(mockEnvironment),
    getGitStatus: vi.fn().mockResolvedValue({
      currentBranch: 'main',
      baseBranch: 'main',
      hasUncommittedChanges: false,
      hasConflicts: false,
      status: 'up_to_date',
      commitsAhead: 0,
      commitsBehind: 0,
      originExists: true,
    }),
    getGitChanges: vi.fn().mockResolvedValue({
      currentBranch: 'main',
      status: 'clean',
      additions: [],
      deletions: [],
      modifications: [],
    }),
    getCurrentCommitHash: vi.fn().mockResolvedValue('abc123'),
    ...overrides,
  } as unknown as Workspace
}

export function createMockWorkspaceWithInvalidPath(overrides?: Partial<Workspace>): Workspace {
  return createMockWorkspace({
    isWorkspacePathValid: vi.fn().mockResolvedValue(false),
    ...overrides,
  })
}

export function createMockDockerWorkspace(overrides?: Partial<Workspace>): Workspace {
  const dockerEnvironment = createMockEnvironment({
    getType: vi.fn().mockReturnValue('docker'),
  })

  return createMockWorkspace({
    type: 'docker',
    getEnvironment: vi.fn().mockReturnValue(dockerEnvironment),
    ...overrides,
  })
}

/**
 * Smart mock that simulates workspace state transitions
 */
export function createSmartMockWorkspace(): Workspace {
  let status = 'idle'
  let isPathValid = true

  const workspace = createMockWorkspace({
    getStatus: vi.fn().mockImplementation(() => status),
    isWorkspacePathValid: vi.fn().mockImplementation(async () => isPathValid),
    setup: vi.fn().mockImplementation(async () => {
      status = 'setting_up'
      // Simulate async setup
      await new Promise((resolve) => setTimeout(resolve, 10))
      status = 'idle'
      isPathValid = true
    }),
  })

  // Add state manipulation methods for testing
  ;(workspace as any).__setStatus = (newStatus: string) => {
    status = newStatus
  }
  ;(workspace as any).__setPathValid = (valid: boolean) => {
    isPathValid = valid
  }

  return workspace
}
