import type { CodeExecution, PreviewConfig } from '../../schemas/repository.schemas.js'
import type {
  BranchListResponse,
  FileSearchResponse,
  RepositoryList,
  RepositoryResponse,
} from '../../types/repository.js'

export function createMockRepository(overrides?: Partial<RepositoryResponse>): RepositoryResponse {
  return {
    id: 'test-repo-123',
    name: 'test-repository',
    description: 'A test repository',
    path: '/tmp/test-repo',
    useGitWorktrees: false,
    cloneOnNewTask: false,
    originalBranch: 'main',
    codeExecution: createMockCodeExecution(),
    createdAt: new Date('2024-01-01T00:00:00Z'),
    updatedAt: new Date('2024-01-01T00:00:00Z'),
    ...overrides,
  }
}

export function createMockCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'local',
    setupCommands: ['npm install'],
    ...overrides,
  }
}

export function createMockDockerCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'docker',
    dockerImage: 'node:18',
    setupCommands: ['npm install'],
    envVars: { NODE_ENV: 'development' },
    volumes: ['/app'],
    ports: { '3000': 3000 },
    volumeMapping: {
      hostPath: '/tmp/test-repo',
      containerPath: '/app',
    },
    resources: {
      cpuLimit: '1',
      memoryLimit: '512Mi',
    },
    ...overrides,
  }
}

export function createMockPreviewConfig(overrides?: Partial<PreviewConfig>): PreviewConfig {
  return {
    serverStartCommands: ['npm run dev'],
    takeScreenshots: false,
    enableInspector: true,
    ...overrides,
  }
}

export function createMockRepositoryWithPreview(
  overrides?: Partial<RepositoryResponse>,
): RepositoryResponse {
  return createMockRepository({
    previewConfig: createMockPreviewConfig(),
    ...overrides,
  })
}

export function createMockDockerRepository(
  overrides?: Partial<RepositoryResponse>,
): RepositoryResponse {
  return createMockRepository({
    codeExecution: createMockDockerCodeExecution(),
    previewConfig: createMockPreviewConfig(),
    ...overrides,
  })
}

export function createMockRepositoryList(repositoryCount = 3): RepositoryList {
  const repositories = Array.from({ length: repositoryCount }, (_, index) =>
    createMockRepository({
      id: `test-repo-${index + 1}`,
      name: `test-repository-${index + 1}`,
      description: `Test repository ${index + 1}`,
    }),
  )

  return { repositories }
}

export function createMockBranchListResponse(
  branches: string[] = ['main', 'develop', 'feature/test'],
): BranchListResponse {
  return {
    branches,
  }
}

export function createMockFileSearchResponse(fileCount = 5): FileSearchResponse {
  const results = Array.from({ length: fileCount }, (_, index) => ({
    path: `src/file${index + 1}.ts`,
    absolutePath: `/tmp/test-repo/src/file${index + 1}.ts`,
    score: 0.9 - index * 0.1,
  }))

  return {
    results,
    totalFound: fileCount,
    query: 'test',
  }
}
