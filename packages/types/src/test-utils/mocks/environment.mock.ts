import type { z } from 'zod'
import type {
  FileTreeNodeType,
  SyncWorkspaceResponseSchema,
} from '../../schemas/workspace.schemas.js'
import type {
  PreviewEnvironmentMetadata,
  PreviewEnvironmentOptions,
  ProcessValidation,
} from '../../types/preview-environment.js'
import type {
  DockerWorkspace,
  FileTreeWithChangesResponse,
  LocalWorkspace,
  Workspace,
  WorkspaceCapabilities,
  WorkspaceStatusResponse,
} from '../../types/workspace.js'

type SyncWorkspaceResponse = z.infer<typeof SyncWorkspaceResponseSchema>

export function createMockLocalWorkspace(overrides?: Partial<LocalWorkspace>): LocalWorkspace {
  return {
    id: 'local-workspace-123',
    type: 'local',
    repositoryId: 'test-repo',
    status: 'ready',
    createdAt: new Date('2024-01-01T00:00:00Z'),
    path: '/tmp/test-workspace',
    branch: 'feature/test',
    originalBranch: 'main',
    ...overrides,
  }
}

export function createMockDockerWorkspace(overrides?: Partial<DockerWorkspace>): DockerWorkspace {
  return {
    id: 'docker-workspace-123',
    type: 'docker',
    repositoryId: 'test-repo',
    status: 'ready',
    createdAt: new Date('2024-01-01T00:00:00Z'),
    containerName: 'test-container-123',
    port: 8080,
    image: 'node:18',
    ...overrides,
  }
}

export function createMockWorkspace(type: 'local' | 'docker' = 'local'): Workspace {
  return type === 'local' ? createMockLocalWorkspace() : createMockDockerWorkspace()
}

export function createMockWorkspaceWithError(error: string): Workspace {
  return createMockLocalWorkspace({
    status: 'error',
    error,
  })
}

export function createMockWorkspaceCapabilities(
  overrides?: Partial<WorkspaceCapabilities>,
): WorkspaceCapabilities {
  return {
    canListFiles: true,
    canReadWriteFiles: true,
    canSearchFiles: true,
    canExecuteCode: true,
    ...overrides,
  }
}

export function createMockWorkspaceStatusResponse(
  overrides?: Partial<WorkspaceStatusResponse>,
): WorkspaceStatusResponse {
  return {
    // Workspace info
    workspaceState: 'exists',
    workspacePath: '/tmp/test-workspace',
    workspaceChangeStatus: 'no_changes',

    // Environment info
    environmentType: 'local',
    environmentStatus: 'ready',
    environmentCreatedAt: '2024-01-01T00:00:00Z',

    // Repository config info
    configOutdated: false,

    // Agent info
    agentState: 'idle',
    agentType: 'claude-code',
    capabilities: {
      fileOperations: true,
      commandExecution: true,
      serverManagement: true,
    },
    gitStatus: {
      currentBranch: 'feature/test',
      baseBranch: 'main',
      hasUncommittedChanges: true,
      hasConflicts: false,
      status: 'ahead',
      commitsAhead: 2,
      commitsBehind: 0,
      originExists: true,
      additions: 15,
      deletions: 5,
      modifications: 3,
    },

    // Computed states
    isRunning: false,
    workspaceReady: true,
    canExecute: true,

    ...overrides,
  }
}

export function createMockFileTreeWithChanges(fileCount = 5): FileTreeWithChangesResponse {
  const tree = Array.from({ length: fileCount }, (_, index) => {
    const node: FileTreeNodeType = {
      name: `file${index + 1}.ts`,
      path: `src/file${index + 1}.ts`,
      type: 'file' as const,
      isChanged: index % 2 === 0,
      isStaged: false,
    }

    if (index % 2 === 0) {
      node.gitStatus = 'modified' as const
      node.additions = 5
      node.deletions = 2
    }

    return node
  })

  return {
    tree,
    gitSummary: {
      totalFiles: fileCount,
      totalAdditions: Math.floor(fileCount / 2) * 5,
      totalDeletions: Math.floor(fileCount / 2) * 2,
      totalModified: Math.floor(fileCount / 2),
      totalStaged: 0,
      compareBase: 'main',
      hasChanges: true,
    },
  }
}

export function createMockSyncWorkspaceResponse(
  overrides?: Partial<SyncWorkspaceResponse>,
): SyncWorkspaceResponse {
  return {
    success: true,
    conflicts: false,
    message: 'Workspace synchronized successfully',
    commitsBehind: 0,
    commitsAhead: 1,
    ...overrides,
  }
}

export function createMockSyncWorkspaceWithConflicts(): SyncWorkspaceResponse {
  return createMockSyncWorkspaceResponse({
    success: false,
    conflicts: true,
    message: 'Conflicts detected during sync',
    filesWithConflicts: ['src/main.ts', 'package.json'],
    unmergedPaths: ['src/main.ts'],
  })
}

// Preview Environment Mocks

export function createMockPreviewEnvironmentMetadata(
  overrides?: Partial<PreviewEnvironmentMetadata>,
): PreviewEnvironmentMetadata {
  return {
    taskId: 'task-123',
    pid: 12345,
    port: 3000,
    startTime: new Date('2024-01-01T00:00:00Z'),
    command: 'npm run dev',
    workspacePath: '/tmp/test-workspace',
    logFile: '/tmp/test-workspace/.logs/preview.log',
    processStartTime: Date.now(),
    envVars: {
      NODE_ENV: 'development',
      PORT: '3000',
    },
    ...overrides,
  }
}

export function createMockPreviewEnvironmentWithError(error: string): PreviewEnvironmentMetadata {
  return createMockPreviewEnvironmentMetadata({
    error,
    failed: true,
    logs: `Error starting preview server: ${error}`,
  })
}

export function createMockPreviewEnvironmentOptions(
  overrides?: Partial<PreviewEnvironmentOptions>,
): PreviewEnvironmentOptions {
  return {
    timeout: 30000,
    forceKill: false,
    validateProcess: true,
    ...overrides,
  }
}

export function createMockProcessValidation(
  overrides?: Partial<ProcessValidation>,
): ProcessValidation {
  return {
    exists: true,
    isCorrectProcess: true,
    command: 'npm run dev',
    startTime: Date.now() - 60000, // Started 1 minute ago
    ...overrides,
  }
}

export function createMockProcessValidationFailed(): ProcessValidation {
  return createMockProcessValidation({
    exists: false,
    isCorrectProcess: false,
  })
}
