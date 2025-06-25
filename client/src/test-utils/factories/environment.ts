import type {
  CodeExecution,
  CreateEnvironmentRequest,
  EnvironmentResponse,
  GithubRepository,
  LocalRepository,
  UpdateEnvironmentRequest,
} from '@shared/types/environment'

// Repository factories
export function createMockLocalRepository(overrides?: Partial<LocalRepository>): LocalRepository {
  return {
    type: 'local',
    path: '/Users/developer/projects/test-app',
    useGitWorktrees: false,
    ...overrides,
  }
}

export function createMockGithubRepository(
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

// Environment factories
export function createMockEnvironment(
  overrides?: Partial<EnvironmentResponse>,
): EnvironmentResponse {
  return {
    id: 'env-123',
    name: 'Test Environment',
    description: 'A test environment for unit tests',
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

export function createMockGithubEnvironment(
  overrides?: Partial<EnvironmentResponse>,
): EnvironmentResponse {
  return {
    id: 'env-github-456',
    name: 'GitHub Environment',
    description: 'A GitHub-based test environment',
    repository: createMockGithubRepository(),
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
export function createMockCreateEnvironmentRequest(
  overrides?: Partial<CreateEnvironmentRequest>,
): CreateEnvironmentRequest {
  return {
    name: 'New Test Environment',
    description: 'A newly created test environment',
    repository: createMockLocalRepository(),
    codeExecution: createMockLocalCodeExecution(),
    variables: {
      NODE_ENV: 'development',
    },
    ...overrides,
  }
}

export function createMockUpdateEnvironmentRequest(
  overrides?: Partial<UpdateEnvironmentRequest>,
): UpdateEnvironmentRequest {
  return {
    name: 'Updated Environment Name',
    description: 'Updated environment description',
    ...overrides,
  }
}

// List factory
export function createMockEnvironmentList(count = 3): EnvironmentResponse[] {
  const environments: EnvironmentResponse[] = []

  for (let i = 0; i < count; i++) {
    const isGithub = i % 2 === 1
    environments.push(
      isGithub
        ? createMockGithubEnvironment({
            id: `env-github-${i}`,
            name: `GitHub Environment ${i}`,
          })
        : createMockEnvironment({
            id: `env-local-${i}`,
            name: `Local Environment ${i}`,
          }),
    )
  }

  return environments
}
