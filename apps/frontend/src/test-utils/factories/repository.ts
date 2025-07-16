import type {
  CodeExecution,
  CreateRepositoryRequest,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/types'

// Code execution factories
export function createMockLocalCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'local',
    setupCommands: ['npm install', 'npm run build'],
    ...overrides,
  }
}

export function createMockDockerCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'docker',
    dockerImage: 'node:18-alpine',
    setupCommands: ['npm ci'],
    envVars: {
      NODE_ENV: 'production',
    },
    volumes: undefined,
    ports: {
      '3000': 3000,
    },
    ...overrides,
  }
}

// Repository factories
export function createMockRepository(overrides?: Partial<RepositoryResponse>): RepositoryResponse {
  return {
    id: 'env-123',
    name: 'Test Repository',
    description: 'A test repository for unit tests',
    path: '/Users/developer/projects/test-app',
    useGitWorktrees: false,
    codeExecution: createMockLocalCodeExecution(),
    createdAt: new Date('2024-01-01T00:00:00Z'),
    updatedAt: new Date('2024-01-01T00:00:00Z'),
    ...overrides,
  }
}

// Request factories
export function createMockCreateRepositoryRequest(
  overrides?: Partial<CreateRepositoryRequest>,
): CreateRepositoryRequest {
  return {
    name: 'New Test Repository',
    description: 'A newly created test repository',
    path: '/Users/developer/projects/test-app',
    useGitWorktrees: false,
    codeExecution: createMockLocalCodeExecution(),
    ...overrides,
  }
}

export function createMockUpdateRepositoryRequest(
  overrides?: Partial<UpdateRepositoryRequest>,
): UpdateRepositoryRequest {
  return {
    name: 'Updated Repository Name',
    description: 'Updated repository description',
    ...overrides,
  }
}

// List factory
export function createMockRepositoryList(count = 3): RepositoryResponse[] {
  const repositories: RepositoryResponse[] = []

  for (let i = 0; i < count; i++) {
    repositories.push(
      createMockRepository({
        id: `repo-local-${i}`,
        name: `Local Repository ${i}`,
      }),
    )
  }

  return repositories
}
