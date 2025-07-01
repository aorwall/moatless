import type {
  CodeExecution,
  CreateRepositoryRequest,
  Repository,
  RepositoryResponse,
  UpdateRepositoryRequest,
} from '@moatless/shared/types/repository'

type LocalRepository = Extract<Repository, { type: 'local' }>
type GithubRepository = Extract<Repository, { type: 'github' }>

// Repository factories
export function createMockLocalRepository(overrides?: Partial<LocalRepository>): LocalRepository {
  return {
    type: 'local',
    path: '/Users/developer/projects/test-app',
    useGitWorktrees: false,
    ...overrides,
  }
}

export function createMockGithubRepositoryConfig(
  overrides?: Partial<GithubRepository>,
): GithubRepository {
  return {
    type: 'github',
    url: 'https://github.com/test-org/test-repo',
    repoId: 'test-org/test-repo',
    branch: 'main',
    commit: undefined,
    token: undefined,
    ...overrides,
  }
}

// Code execution factories
export function createMockLocalCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'local',
    setupCommands: 'npm install\nnpm run build',
    ...overrides,
  }
}

export function createMockDockerCodeExecution(overrides?: Partial<CodeExecution>): CodeExecution {
  return {
    type: 'docker',
    image: 'node:18-alpine',
    setupCommands: 'npm ci',
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
    repository: createMockLocalRepository(),
    codeExecution: createMockLocalCodeExecution(),
    variables: {
      NODE_ENV: 'test',
      API_URL: 'http://localhost:3000',
    },
    createdAt: new Date('2024-01-01T00:00:00Z'),
    updatedAt: new Date('2024-01-01T00:00:00Z'),
    ...overrides,
  }
}

export function createMockGithubRepository(
  overrides?: Partial<RepositoryResponse>,
): RepositoryResponse {
  return {
    id: 'env-github-456',
    name: 'GitHub Repository',
    description: 'A GitHub-based test repository',
    repository: createMockGithubRepositoryConfig(),
    codeExecution: createMockDockerCodeExecution(),
    variables: {
      NODE_ENV: 'production',
      DATABASE_URL: 'postgresql://localhost:5432/testdb',
    },
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
    repository: createMockLocalRepository(),
    codeExecution: createMockLocalCodeExecution(),
    variables: {
      NODE_ENV: 'development',
    },
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
    const isGithub = i % 2 === 1
    repositories.push(
      isGithub
        ? createMockGithubRepository({
            id: `repo-github-${i}`,
            name: `GitHub Repository ${i}`,
          })
        : createMockRepository({
            id: `repo-local-${i}`,
            name: `Local Repository ${i}`,
          }),
    )
  }

  return repositories
}
